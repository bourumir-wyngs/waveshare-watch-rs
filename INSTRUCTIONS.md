# 🦀 Waveshare ESP32-S3 AMOLED Watch — Rust Build & Flash (Ubuntu)

## 📦 1. Install system dependencies

` sudo apt update
` sudo apt install -y git curl build-essential pkg-config libudev-dev

---

## 🦀 2. Install Rust

` curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
` . "`HOME/.cargo/env"

---

## ⚙️ 3. Install ESP toolchain (Xtensa)

` cargo install espup --locked
` espup install

# Load environment (IMPORTANT)
` . "`HOME/export-esp.sh"

Verify toolchain:

` which xtensa-esp32s3-elf-gcc

---

## 🔌 4. Install flashing tool

` cargo install espflash

---

## 📥 5. Clone project

` git clone https://github.com/infinition/waveshare-watch-rs.git
` cd waveshare-watch-rs

---

## 📡 6. Configure WiFi (compile-time)

` export WIFI_SSID="your-wifi-name"
` export WIFI_PASS="your-wifi-password"

---

## 🏗️ 7. Build firmware

` . "`HOME/export-esp.sh"
` cargo build --release

Default build includes the core watch firmware and no apps. Enable apps explicitly:

` cargo build --release --no-default-features --features "snake tetris"

Full app build:

` cargo build --release --no-default-features --features "apps ble audio"

Output binary:

` target/xtensa-esp32s3-none-elf/release/waveshare-watch-rs

---

## 🔍 8. Find serial port

` ls -l /dev/ttyACM* /dev/ttyUSB* 2>/dev/null
` espflash board-info

Typical:
- /dev/ttyACM0
- /dev/ttyUSB0

---

## ⚡ 9. Flash + monitor

` espflash flash \
  --port /dev/ttyACM0 \
  --monitor \
  target/xtensa-esp32s3-none-elf/release/waveshare-watch-rs

---

## 🔐 10. Fix permissions (if needed)

` sudo usermod -aG dialout "`USER"
` newgrp dialout

Unplug + replug device.

---

## 🔁 11. Manual bootloader mode (if flashing fails)

Hold BOOT
Tap RESET
Release BOOT

Then retry flashing.

---

## 🧠 12. Make environment persistent

` echo '. "`HOME/export-esp.sh"' >> ~/.bashrc

---

## ✅ Done

You now have:
- no_std Rust firmware
- Running on ESP32-S3 smartwatch
- Fully flashed from Ubuntu

---

## 🚀 Next steps (optional)

- Display rendering pipeline (AMOLED driver internals)
- Touch input handling (FT3168)
- BLE / WiFi features
- Power management (deep sleep)
- Custom firmware development
- Serial/JTAG debugging
