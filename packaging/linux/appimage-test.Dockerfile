FROM ubuntu:24.04
# Only the host graphics stack, a font, and test tools. Do not install the
# FerryDock .deb or its application dependencies in this independent image.
RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 xvfb xdotool imagemagick tesseract-ocr tesseract-ocr-eng \
    fonts-dejavu-core libvulkan1 mesa-vulkan-drivers libegl1 libgl1-mesa-dri \
    libwayland-client0 libwayland-cursor0 libwayland-egl1 \
    && rm -rf /var/lib/apt/lists/*
