use crate::starry_config::{SecureTcpConfig, SecureTcpMode};
use hbb_common::{
    bytes::{Bytes, BytesMut},
    bytes_codec::BytesCodec,
    futures_util::{
        sink::SinkExt,
        stream::{SplitSink, SplitStream, StreamExt},
    },
    protobuf::Message as _,
    rendezvous_proto::{rendezvous_message, KeyExchange, RendezvousMessage},
    timeout,
    tokio::net::TcpStream,
    tokio_util::codec::Framed,
};
use sodiumoxide::crypto::{box_, secretbox, sign};
use std::io::{self, Error, ErrorKind};

type TcpFramed = Framed<TcpStream, BytesCodec>;
type RawSink = SplitSink<TcpFramed, Bytes>;
type RawStream = SplitStream<TcpFramed>;

pub(crate) enum TcpWriteTransport {
    Plain(RawSink),
    Secure { sink: RawSink, cipher: SecureCipher },
}

pub(crate) enum TcpReadTransport {
    Plain(RawStream),
    Secure {
        stream: RawStream,
        cipher: SecureCipher,
    },
}

pub(crate) struct NegotiatedTcp {
    pub(crate) sink: TcpWriteTransport,
    pub(crate) stream: TcpReadTransport,
    pub(crate) first_plaintext: Option<BytesMut>,
    pub(crate) secured: bool,
}

impl TcpWriteTransport {
    pub(crate) async fn send(&mut self, bytes: Bytes) -> io::Result<()> {
        match self {
            Self::Plain(sink) => sink.send(bytes).await,
            Self::Secure { sink, cipher } => {
                let encrypted = cipher.encrypt(&bytes)?;
                sink.send(Bytes::from(encrypted)).await
            }
        }
    }
}

impl TcpReadTransport {
    pub(crate) async fn next(&mut self) -> Option<io::Result<BytesMut>> {
        match self {
            Self::Plain(stream) => stream.next().await,
            Self::Secure { stream, cipher } => match stream.next().await {
                Some(Ok(bytes)) => Some(
                    cipher
                        .decrypt(&bytes)
                        .map(|plaintext| BytesMut::from(plaintext.as_slice())),
                ),
                Some(Err(err)) => Some(Err(err)),
                None => None,
            },
        }
    }
}

pub(crate) async fn negotiate(
    stream: TcpStream,
    signing_key: Option<&sign::SecretKey>,
    config: &SecureTcpConfig,
) -> io::Result<NegotiatedTcp> {
    let mut codec = BytesCodec::new();
    if config.mode == SecureTcpMode::Auto && signing_key.is_some() {
        codec.set_max_packet_length(config.max_frame_bytes);
    }
    let mut framed = Framed::new(stream, codec);

    if config.mode == SecureTcpMode::Off || signing_key.is_none() {
        return Ok(plain_transport(framed, None));
    }
    let signing_key = signing_key.ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            "Secure TCP requires a server signing key",
        )
    })?;
    let (curve_public_key, curve_secret_key) = signing_keypair(signing_key)?;
    send_server_key_exchange(&mut framed, signing_key, &curve_public_key).await?;

    let first = match timeout(config.handshake_timeout_ms, framed.next()).await {
        Ok(Some(Ok(bytes))) => bytes,
        Ok(Some(Err(err))) => return Err(err),
        Ok(None) => {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "connection closed during Secure TCP negotiation",
            ))
        }
        Err(_) => {
            return Err(Error::new(
                ErrorKind::TimedOut,
                "Secure TCP negotiation timed out",
            ))
        }
    };

    let message = RendezvousMessage::parse_from_bytes(&first).map_err(|err| {
        Error::new(
            ErrorKind::InvalidData,
            format!("invalid first Rendezvous frame: {err}"),
        )
    })?;
    match message.union {
        Some(rendezvous_message::Union::KeyExchange(exchange)) => {
            let key = open_client_key(&exchange, &curve_secret_key)?;
            let (sink, stream) = framed.split();
            Ok(NegotiatedTcp {
                sink: TcpWriteTransport::Secure {
                    sink,
                    cipher: SecureCipher::new(key.clone()),
                },
                stream: TcpReadTransport::Secure {
                    stream,
                    cipher: SecureCipher::new(key),
                },
                first_plaintext: None,
                secured: true,
            })
        }
        _ => Ok(plain_transport(framed, Some(first))),
    }
}

fn plain_transport(framed: TcpFramed, first_plaintext: Option<BytesMut>) -> NegotiatedTcp {
    let (sink, stream) = framed.split();
    NegotiatedTcp {
        sink: TcpWriteTransport::Plain(sink),
        stream: TcpReadTransport::Plain(stream),
        first_plaintext,
        secured: false,
    }
}

fn signing_keypair(
    signing_key: &sign::SecretKey,
) -> io::Result<(box_::PublicKey, box_::SecretKey)> {
    let public_offset = sign::SECRETKEYBYTES - sign::PUBLICKEYBYTES;
    let mut public = [0_u8; sign::PUBLICKEYBYTES];
    public.copy_from_slice(&signing_key.0[public_offset..]);
    let signing_public = sign::PublicKey(public);
    let curve_public = sign::ed25519::to_curve25519_pk(&signing_public).map_err(|_| {
        Error::new(
            ErrorKind::InvalidData,
            "cannot convert the server Ed25519 public key to Curve25519",
        )
    })?;
    let curve_secret = sign::ed25519::to_curve25519_sk(signing_key).map_err(|_| {
        Error::new(
            ErrorKind::InvalidData,
            "cannot convert the server Ed25519 secret key to Curve25519",
        )
    })?;
    Ok((curve_public, curve_secret))
}

async fn send_server_key_exchange(
    framed: &mut TcpFramed,
    signing_key: &sign::SecretKey,
    curve_public_key: &box_::PublicKey,
) -> io::Result<()> {
    let signed_public_key = sign::sign(&curve_public_key.0, signing_key);
    let mut message = RendezvousMessage::new();
    message.set_key_exchange(KeyExchange {
        keys: vec![signed_public_key.into()],
        ..Default::default()
    });
    let bytes = message.write_to_bytes().map_err(|err| {
        Error::new(
            ErrorKind::InvalidData,
            format!("cannot encode server key exchange: {err}"),
        )
    })?;
    framed.send(Bytes::from(bytes)).await
}

fn open_client_key(
    exchange: &KeyExchange,
    curve_secret_key: &box_::SecretKey,
) -> io::Result<secretbox::Key> {
    if exchange.keys.len() != 2 {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "client key exchange must contain exactly two values",
        ));
    }
    if exchange.keys[0].len() != box_::PUBLICKEYBYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "client Curve25519 public key has an invalid length",
        ));
    }
    if exchange.keys[1].len() != secretbox::KEYBYTES + box_::MACBYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "sealed client symmetric key has an invalid length",
        ));
    }

    let mut public = [0_u8; box_::PUBLICKEYBYTES];
    public.copy_from_slice(&exchange.keys[0]);
    let client_public_key = box_::PublicKey(public);
    let nonce = box_::Nonce([0_u8; box_::NONCEBYTES]);
    let opened = box_::open(
        &exchange.keys[1],
        &nonce,
        &client_public_key,
        curve_secret_key,
    )
    .map_err(|_| {
        Error::new(
            ErrorKind::InvalidData,
            "cannot authenticate the client symmetric key",
        )
    })?;
    if opened.len() != secretbox::KEYBYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "client symmetric key has an invalid length",
        ));
    }
    secretbox::Key::from_slice(&opened).ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidData,
            "cannot construct the client symmetric key",
        )
    })
}

pub(crate) struct SecureCipher {
    key: secretbox::Key,
    sequence: u64,
}

impl SecureCipher {
    fn new(key: secretbox::Key) -> Self {
        Self { key, sequence: 0 }
    }

    fn encrypt(&mut self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        let nonce = self.next_nonce()?;
        Ok(secretbox::seal(plaintext, &nonce, &self.key))
    }

    fn decrypt(&mut self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
        if ciphertext.len() < secretbox::MACBYTES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "encrypted frame is shorter than the authentication tag",
            ));
        }
        let nonce = self.next_nonce()?;
        secretbox::open(ciphertext, &nonce, &self.key).map_err(|_| {
            Error::new(
                ErrorKind::InvalidData,
                "Secure TCP frame authentication failed",
            )
        })
    }

    fn next_nonce(&mut self) -> io::Result<secretbox::Nonce> {
        self.sequence = self.sequence.checked_add(1).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                "Secure TCP nonce sequence exhausted",
            )
        })?;
        let mut nonce = secretbox::Nonce([0_u8; secretbox::NONCEBYTES]);
        nonce.0[..std::mem::size_of::<u64>()].copy_from_slice(&self.sequence.to_le_bytes());
        Ok(nonce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::{
        rendezvous_proto::TestNatRequest,
        tokio::{net::TcpListener, task},
    };

    fn auto_config() -> SecureTcpConfig {
        SecureTcpConfig {
            mode: SecureTcpMode::Auto,
            handshake_timeout_ms: 5_000,
            idle_timeout_ms: 5_000,
            max_frame_bytes: 65_536,
        }
    }

    #[test]
    fn client_key_exchange_matches_the_rustdesk_protocol() {
        sodiumoxide::init().unwrap();
        let (_signing_public, signing_secret) = sign::gen_keypair();
        let (server_public, server_secret) = signing_keypair(&signing_secret).unwrap();
        let (client_public, client_secret) = box_::gen_keypair();
        let expected = secretbox::gen_key();
        let nonce = box_::Nonce([0_u8; box_::NONCEBYTES]);
        let sealed = box_::seal(&expected.0, &nonce, &server_public, &client_secret);
        let exchange = KeyExchange {
            keys: vec![client_public.0.to_vec().into(), sealed.into()],
            ..Default::default()
        };
        let opened = open_client_key(&exchange, &server_secret).unwrap();
        assert_eq!(opened.0, expected.0);
    }

    #[test]
    fn send_and_receive_counters_are_independent_and_start_at_one() {
        sodiumoxide::init().unwrap();
        let key = secretbox::gen_key();
        let mut sender = SecureCipher::new(key.clone());
        let mut receiver = SecureCipher::new(key);
        let first = sender.encrypt(b"first").unwrap();
        let second = sender.encrypt(b"second").unwrap();
        assert_eq!(receiver.decrypt(&first).unwrap(), b"first");
        assert_eq!(receiver.decrypt(&second).unwrap(), b"second");
    }

    #[test]
    fn authentication_failure_does_not_fall_back_to_plaintext() {
        sodiumoxide::init().unwrap();
        let mut sender = SecureCipher::new(secretbox::gen_key());
        let mut receiver = SecureCipher::new(secretbox::gen_key());
        let encrypted = sender.encrypt(b"payload").unwrap();
        assert!(receiver.decrypt(&encrypted).is_err());
    }

    #[test]
    fn negotiates_and_exchanges_encrypted_frames_over_real_tcp() {
        let runtime = hbb_common::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            sodiumoxide::init().unwrap();
            let (signing_public, signing_secret) = sign::gen_keypair();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = task::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut negotiated = negotiate(stream, Some(&signing_secret), &auto_config())
                    .await
                    .unwrap();
                assert!(negotiated.secured);
                assert!(negotiated.first_plaintext.is_none());
                let request = negotiated.stream.next().await.unwrap().unwrap();
                assert_eq!(&request[..], b"secure-request");
                negotiated
                    .sink
                    .send(Bytes::from_static(b"secure-response"))
                    .await
                    .unwrap();
            });

            let stream = TcpStream::connect(address).await.unwrap();
            let mut client = Framed::new(stream, BytesCodec::new());
            let offer = client.next().await.unwrap().unwrap();
            let offer = RendezvousMessage::parse_from_bytes(&offer).unwrap();
            let exchange = match offer.union {
                Some(rendezvous_message::Union::KeyExchange(exchange)) => exchange,
                _ => panic!("server did not send a key exchange offer"),
            };
            assert_eq!(exchange.keys.len(), 1);
            let verified = sign::verify(&exchange.keys[0], &signing_public).unwrap();
            let mut server_curve_public = [0_u8; box_::PUBLICKEYBYTES];
            server_curve_public.copy_from_slice(&verified);
            let server_curve_public = box_::PublicKey(server_curve_public);
            let (client_public, client_secret) = box_::gen_keypair();
            let key = secretbox::gen_key();
            let nonce = box_::Nonce([0_u8; box_::NONCEBYTES]);
            let sealed = box_::seal(&key.0, &nonce, &server_curve_public, &client_secret);
            let mut response = RendezvousMessage::new();
            response.set_key_exchange(KeyExchange {
                keys: vec![client_public.0.to_vec().into(), sealed.into()],
                ..Default::default()
            });
            client
                .send(Bytes::from(response.write_to_bytes().unwrap()))
                .await
                .unwrap();

            let mut client_sender = SecureCipher::new(key.clone());
            let encrypted = client_sender.encrypt(b"secure-request").unwrap();
            client.send(Bytes::from(encrypted)).await.unwrap();
            let encrypted_response = client.next().await.unwrap().unwrap();
            let mut client_receiver = SecureCipher::new(key);
            assert_eq!(
                client_receiver.decrypt(&encrypted_response).unwrap(),
                b"secure-response"
            );
            server.await.unwrap();
        });
    }

    #[test]
    fn valid_plaintext_first_frame_keeps_upstream_transport() {
        let runtime = hbb_common::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            sodiumoxide::init().unwrap();
            let (_signing_public, signing_secret) = sign::gen_keypair();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = task::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut negotiated = negotiate(stream, Some(&signing_secret), &auto_config())
                    .await
                    .unwrap();
                assert!(!negotiated.secured);
                let first = negotiated.first_plaintext.take().unwrap();
                let message = RendezvousMessage::parse_from_bytes(&first).unwrap();
                assert!(matches!(
                    message.union,
                    Some(rendezvous_message::Union::TestNatRequest(_))
                ));
                negotiated
                    .sink
                    .send(Bytes::from_static(b"plain-response"))
                    .await
                    .unwrap();
            });

            let stream = TcpStream::connect(address).await.unwrap();
            let mut client = Framed::new(stream, BytesCodec::new());
            let offer = client.next().await.unwrap().unwrap();
            let offer = RendezvousMessage::parse_from_bytes(&offer).unwrap();
            assert!(matches!(
                offer.union,
                Some(rendezvous_message::Union::KeyExchange(_))
            ));

            let mut first = RendezvousMessage::new();
            first.set_test_nat_request(TestNatRequest::new());
            client
                .send(Bytes::from(first.write_to_bytes().unwrap()))
                .await
                .unwrap();
            assert_eq!(
                &client.next().await.unwrap().unwrap()[..],
                b"plain-response"
            );
            server.await.unwrap();
        });
    }
}
