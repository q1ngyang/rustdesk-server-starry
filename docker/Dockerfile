FROM alpine:3.22@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce

RUN apk add --no-cache ca-certificates=20260611-r0

ARG TARGETARCH
COPY --chmod=755 ${TARGETARCH}/hbbs /usr/bin/hbbs
COPY --chmod=755 ${TARGETARCH}/hbbr /usr/bin/hbbr
COPY --chmod=755 ${TARGETARCH}/rustdesk-utils /usr/bin/rustdesk-utils
COPY --chmod=755 ${TARGETARCH}/starry-control-agent /usr/bin/starry-control-agent

ENV HOME=/root

WORKDIR /root
VOLUME ["/root"]

EXPOSE 21115 21116 21116/udp 21117 21118 21119

CMD ["hbbs", "--starry-config=/root/starry/config.yaml"]
