//! BLE advertising control via raw HCI commands.
//!
//! esp-radio's `BleConnector` is a low-level HCI transport — it doesn't
//! provide a GATT server or high-level advertising API. For the smartwatch
//! we only need the device to be discoverable (advertising its name), so
//! we send 3 HCI commands directly:
//!
//!   1. LE Set Advertising Parameters (slow interval = power-friendly)
//!   2. LE Set Advertising Data (Flags + Complete Local Name)
//!   3. LE Set Advertising Enable (on / off)
//!
//! The VHCI interface in ESP-IDF expects H4 transport framing, so every
//! command is prefixed with 0x01 (HCI Command Packet).

use embedded_io::Write;

/// Start BLE advertising as "Rust Watch".
/// Sends HCI commands synchronously via the BleConnector's Write impl.
pub fn start_advertising<W: Write>(hci: &mut W) -> Result<(), W::Error> {
    // 1) LE Set Advertising Parameters
    //    Opcode 0x2006, 15 bytes of params
    //    Interval: 0x0800 (1.28s) — slow to save power
    //    Type: ADV_IND (connectable, undirected)
    //    Channels: all 3 (37, 38, 39)
    hci.write_all(&[
        0x01, // H4: HCI command
        0x06, 0x20, // opcode: LE Set Advertising Parameters
        15,   // param length
        0x00, 0x08, // interval min: 0x0800 (1280 * 0.625ms = 800ms)
        0x00, 0x08, // interval max: 0x0800
        0x00, // type: ADV_IND
        0x00, // own addr type: public
        0x00, // peer addr type
        0, 0, 0, 0, 0, 0,    // peer addr (unused)
        0x07, // channel map: all
        0x00, // filter policy: any
    ])?;

    // 2) LE Set Advertising Data
    //    Opcode 0x2008, always 32 bytes of param (1 len + 31 data)
    let name = b"Rust Watch";
    let flags_len: u8 = 3; // AD: [len=2, type=0x01 Flags, val=0x06]
    let name_ad_len: u8 = 1 + name.len() as u8; // [type + name bytes]
    let sig_octets = flags_len + 1 + name_ad_len; // total significant

    let mut cmd = [0u8; 36]; // 1 (H4) + 2 (opcode) + 1 (plen) + 32 (data) = 36
    cmd[0] = 0x01; // H4
    cmd[1] = 0x08;
    cmd[2] = 0x20; // opcode: LE Set Advertising Data
    cmd[3] = 32; // param length (always 32)
    cmd[4] = sig_octets; // significant octets count
                         // Flags AD structure
    cmd[5] = 2; // length of this AD
    cmd[6] = 0x01; // AD type: Flags
    cmd[7] = 0x06; // General Discoverable + BR/EDR Not Supported
                   // Complete Local Name AD structure
    cmd[8] = name_ad_len;
    cmd[9] = 0x09; // AD type: Complete Local Name
    cmd[10..10 + name.len()].copy_from_slice(name);
    // Remaining bytes are zero (padding)
    hci.write_all(&cmd)?;

    // 3) LE Set Advertising Enable
    //    Opcode 0x200A, 1 byte param = 0x01 (enable)
    hci.write_all(&[
        0x01, // H4
        0x0A, 0x20, // opcode: LE Set Advertising Enable
        1,    // param length
        0x01, // enable
    ])?;

    Ok(())
}

/// Stop BLE advertising.
pub fn stop_advertising<W: Write>(hci: &mut W) -> Result<(), W::Error> {
    hci.write_all(&[
        0x01, // H4
        0x0A, 0x20, // opcode: LE Set Advertising Enable
        1,    // param length
        0x00, // disable
    ])?;
    Ok(())
}
