FROM alpine:3.22

RUN apk add --no-cache ca-certificates

ARG TARGETARCH
COPY --chmod=755 ${TARGETARCH}/hbbs /usr/bin/hbbs
COPY --chmod=755 ${TARGETARCH}/hbbr /usr/bin/hbbr
COPY --chmod=755 ${TARGETARCH}/rustdesk-utils /usr/bin/rustdesk-utils

ENV HOME=/root

WORKDIR /root
VOLUME ["/root"]

EXPOSE 21115 21116 21116/udp 21117 21118 21119

CMD ["hbbs", "--starry-config=/root/starry/config.yaml"]
