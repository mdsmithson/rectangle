FROM alpine:3.24
RUN apk add --no-cache firefox tigervnc openbox socat font-dejavu ca-certificates pulseaudio pulseaudio-utils jq python3 xclip xdotool \
    && addgroup -g 10001 rectangle && adduser -D -H -u 10001 -G rectangle rectangle \
    && mkdir -p /profile/data && touch /profile/data/.rectangle && chown -R 10001:10001 /profile/data
# Pin Mozilla's signed release so new profiles can install it without internet access.
RUN mkdir -p /opt/rectangle \
    && wget -q -O /opt/rectangle/ublock-origin.xpi https://addons.mozilla.org/firefox/downloads/file/5225273/ublock_origin-1.76.0.xpi \
    && echo '6f20ff6749193599548770539103069c20948dfcd3e61c3089a67da7bfc3be39  /opt/rectangle/ublock-origin.xpi' | sha256sum -c -
COPY --chmod=0444 firefox-policies.json /usr/lib/firefox/distribution/policies.json
COPY --chmod=0444 rectangle-autoconfig.js /usr/lib/firefox/defaults/pref/rectangle-autoconfig.js
COPY --chmod=0444 rectangle.cfg /usr/lib/firefox/rectangle.cfg
COPY --chmod=0444 navigation-stream.py /opt/rectangle/navigation-stream.py
COPY --chmod=0555 entrypoint /opt/rectangle/entrypoint
COPY --chmod=0555 audio-stream /opt/rectangle/audio-stream
COPY --chmod=0444 clipboard-stream.py /opt/rectangle/clipboard-stream.py
COPY --chmod=0444 memory-stream.py /opt/rectangle/memory-stream.py
COPY --chmod=0444 demo.html /opt/rectangle/demo.html
ENV HOME=/tmp/home DISPLAY=:99
USER 10001:10001
ENTRYPOINT ["/opt/rectangle/entrypoint"]
