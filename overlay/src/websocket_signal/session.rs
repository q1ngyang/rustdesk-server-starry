use hbb_common::{
    futures_util::{stream::SplitSink, SinkExt},
    tokio::{
        net::TcpStream,
        sync::mpsc::{self, error::TrySendError},
    },
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio_tungstenite::WebSocketStream;
use tungstenite::Message;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum SendError {
    Closed,
    Full,
}

pub(crate) enum OutboundFrame {
    Binary(Vec<u8>),
    Pong(Vec<u8>),
    Close,
}

#[derive(Clone)]
pub(crate) struct WsWriteTransport {
    tx: mpsc::Sender<OutboundFrame>,
    closed: Arc<AtomicBool>,
    pub(crate) connection_id: u64,
}

impl WsWriteTransport {
    pub(super) fn channel(
        connection_id: u64,
        capacity: usize,
    ) -> (Self, mpsc::Receiver<OutboundFrame>) {
        let (tx, rx) = mpsc::channel(capacity);
        (
            Self {
                tx,
                closed: Arc::new(AtomicBool::new(false)),
                connection_id,
            },
            rx,
        )
    }

    pub(crate) fn send_binary(&self, bytes: Vec<u8>) -> Result<(), SendError> {
        self.try_send(OutboundFrame::Binary(bytes))
    }

    pub(crate) fn send_pong(&self, bytes: Vec<u8>) -> Result<(), SendError> {
        self.try_send(OutboundFrame::Pong(bytes))
    }

    fn try_send(&self, frame: OutboundFrame) -> Result<(), SendError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(SendError::Closed);
        }
        match self.tx.try_send(frame) {
            Ok(()) => Ok(()),
            Err(TrySendError::Closed(_)) => {
                self.closed.store(true, Ordering::Release);
                Err(SendError::Closed)
            }
            Err(TrySendError::Full(_)) => {
                self.closed.store(true, Ordering::Release);
                let _ = self.tx.try_send(OutboundFrame::Close);
                Err(SendError::Full)
            }
        }
    }

    pub(crate) fn close(&self) {
        if !self.closed.load(Ordering::Acquire) {
            if self.tx.try_send(OutboundFrame::Close).is_err() {
                self.closed.store(true, Ordering::Release);
            }
        }
    }

    pub(crate) fn abort(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            let _ = self.tx.try_send(OutboundFrame::Close);
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

pub(super) async fn writer_loop(
    mut sink: SplitSink<WebSocketStream<TcpStream>, Message>,
    mut rx: mpsc::Receiver<OutboundFrame>,
    closed: WsWriteTransport,
) {
    while let Some(frame) = rx.recv().await {
        if closed.is_closed() && !matches!(frame, OutboundFrame::Close) {
            break;
        }
        let is_close = matches!(frame, OutboundFrame::Close);
        let result = match frame {
            OutboundFrame::Binary(bytes) => sink.send(Message::Binary(bytes.into())).await,
            OutboundFrame::Pong(bytes) => sink.send(Message::Pong(bytes.into())).await,
            OutboundFrame::Close => sink.send(Message::Close(None)).await,
        };
        if result.is_err() || closed.is_closed() || is_close {
            break;
        }
    }
    closed.closed.store(true, Ordering::Release);
    let _ = sink.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_queue_marks_slow_writer_closed() {
        let (transport, _rx) = WsWriteTransport::channel(7, 1);
        assert_eq!(transport.send_binary(vec![1]), Ok(()));
        assert_eq!(transport.send_binary(vec![2]), Err(SendError::Full));
        assert!(transport.is_closed());
    }
}
