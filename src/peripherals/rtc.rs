// PCF85063A RTC driver
// Reference: OLEDS3Watch/components/bsp_extra/src/pcf85063a.c
// I2C address 0x51, BCD encoded time registers

use embedded_hal::i2c::I2c;

const PCF85063A_ADDR: u8 = 0x51;

// Registers
const REG_CTRL1: u8 = 0x00;
const REG_CTRL2: u8 = 0x01;
const REG_RAM_BYTE: u8 = 0x03;
const REG_SECONDS: u8 = 0x04;
const REG_MINUTES: u8 = 0x05;
const REG_HOURS: u8 = 0x06;
const REG_DAYS: u8 = 0x07;
const REG_WEEKDAYS: u8 = 0x08;
const REG_MONTHS: u8 = 0x09;
const REG_YEARS: u8 = 0x0A;
const REG_MINUTE_ALARM: u8 = 0x0B;
const REG_HOUR_ALARM: u8 = 0x0C;
const REG_DAY_ALARM: u8 = 0x0D;
const REG_WEEKDAY_ALARM: u8 = 0x0E;
const REG_TIMER_VAL: u8 = 0x10;
const REG_TIMER_MODE: u8 = 0x11;

#[derive(Debug, Clone, Copy)]
pub struct DateTime {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub day: u8,
    pub weekday: u8,
    pub month: u8,
    pub year: u8, // 0-99 (2000-2099)
}

impl DateTime {
    pub fn new(year: u8, month: u8, day: u8, hours: u8, minutes: u8, seconds: u8) -> Self {
        Self {
            seconds,
            minutes,
            hours,
            day,
            weekday: 0,
            month,
            year,
        }
    }
}

pub struct Pcf85063aRtc<I> {
    i2c: I,
}

impl<I: I2c> Pcf85063aRtc<I> {
    pub fn new(i2c: I) -> Self {
        Self { i2c }
    }

    fn read_reg(&mut self, reg: u8) -> Result<u8, I::Error> {
        let mut buf = [0u8];
        self.i2c.write_read(PCF85063A_ADDR, &[reg], &mut buf)?;
        Ok(buf[0])
    }

    fn write_reg(&mut self, reg: u8, val: u8) -> Result<(), I::Error> {
        self.i2c.write(PCF85063A_ADDR, &[reg, val])
    }

    /// Read the RTC's battery-backed user RAM byte (register 0x03).
    ///
    /// This is a single byte of storage inside the PCF85063A. It is retained
    /// while the RTC remains powered, including across MCU resets/deep sleep,
    /// but is not non-volatile flash/EEPROM.
    pub fn read_ram_byte(&mut self) -> Result<u8, I::Error> {
        self.read_reg(REG_RAM_BYTE)
    }

    /// Write the RTC's battery-backed user RAM byte (register 0x03).
    ///
    /// See [`read_ram_byte`](Self::read_ram_byte) for retention notes.
    pub fn write_ram_byte(&mut self, value: u8) -> Result<(), I::Error> {
        self.write_reg(REG_RAM_BYTE, value)
    }

    /// Set selected bits in the RTC RAM byte, leaving all other bits unchanged.
    pub fn set_ram_flags(&mut self, mask: u8) -> Result<(), I::Error> {
        let value = self.read_ram_byte()?;
        self.write_ram_byte(value | mask)
    }

    /// Clear selected bits in the RTC RAM byte, leaving all other bits unchanged.
    pub fn clear_ram_flags(&mut self, mask: u8) -> Result<(), I::Error> {
        let value = self.read_ram_byte()?;
        self.write_ram_byte(value & !mask)
    }

    /// Read the RTC alarm/alert hour and minute registers.
    ///
    /// Returns `None` if either the minute or hour alarm register is disabled.
    pub fn get_alert_time(&mut self) -> Result<Option<(u8, u8)>, I::Error> {
        let minute_reg = self.read_reg(REG_MINUTE_ALARM)?;
        let hour_reg = self.read_reg(REG_HOUR_ALARM)?;

        if (minute_reg & 0x80) != 0 || (hour_reg & 0x80) != 0 {
            return Ok(None);
        }

        let minutes = bcd_to_dec(minute_reg & 0x7F);
        let hours = bcd_to_dec(hour_reg & 0x3F);
        if hours < 24 && minutes < 60 {
            Ok(Some((hours, minutes)))
        } else {
            Ok(None)
        }
    }

    /// Store an HH:MM value in the RTC alarm/alert registers.
    ///
    /// Only hour and minute matching are enabled; day and weekday alarm matching
    /// are disabled so the register represents a daily time-of-day alert.
    pub fn set_alert_time(&mut self, hours: u8, minutes: u8) -> Result<(), I::Error> {
        let hours = hours.min(23);
        let minutes = minutes.min(59);

        self.write_reg(REG_MINUTE_ALARM, dec_to_bcd(minutes) & 0x7F)?;
        self.write_reg(REG_HOUR_ALARM, dec_to_bcd(hours) & 0x3F)?;
        self.write_reg(REG_DAY_ALARM, 0x80)?;
        self.write_reg(REG_WEEKDAY_ALARM, 0x80)
    }

    /// Disable the RTC alarm/alert time.
    ///
    /// This marks all alarm fields as disabled (bit 7 set) and clears any
    /// pending alarm flag. The stored register values are intentionally not
    /// treated as an active alert while disabled.
    pub fn disable_alert_time(&mut self) -> Result<(), I::Error> {
        self.write_reg(REG_MINUTE_ALARM, 0x80)?;
        self.write_reg(REG_HOUR_ALARM, 0x80)?;
        self.write_reg(REG_DAY_ALARM, 0x80)?;
        self.write_reg(REG_WEEKDAY_ALARM, 0x80)?;

        let ctrl2 = self.read_reg(REG_CTRL2)?;
        self.write_reg(REG_CTRL2, ctrl2 & !(1 << 6))
    }
    /// Initialize RTC: ensure oscillator running, 24h mode.
    pub fn init(&mut self) -> Result<(), I::Error> {
        let ctrl1 = self.read_reg(REG_CTRL1)?;
        // Clear STOP bit (bit 5) to start oscillator
        // Clear 12_24 bit (bit 2) for 24-hour mode
        let new_ctrl1 = ctrl1 & !(0x20 | 0x04);
        if new_ctrl1 != ctrl1 {
            self.write_reg(REG_CTRL1, new_ctrl1)?;
        }
        Ok(())
    }

    /// Read current date/time.
    pub fn get_time(&mut self) -> Result<DateTime, I::Error> {
        // Read all time registers in one burst (7 bytes from 0x04)
        let mut buf = [0u8; 7];
        self.i2c
            .write_read(PCF85063A_ADDR, &[REG_SECONDS], &mut buf)?;

        Ok(DateTime {
            seconds: bcd_to_dec(buf[0] & 0x7F), // mask OS bit
            minutes: bcd_to_dec(buf[1] & 0x7F),
            hours: bcd_to_dec(buf[2] & 0x3F), // 24h mode
            day: bcd_to_dec(buf[3] & 0x3F),
            weekday: buf[4] & 0x07,
            month: bcd_to_dec(buf[5] & 0x1F),
            year: bcd_to_dec(buf[6]),
        })
    }

    /// Set date/time.
    pub fn set_time(&mut self, dt: &DateTime) -> Result<(), I::Error> {
        // Stop oscillator
        let ctrl1 = self.read_reg(REG_CTRL1)?;
        self.write_reg(REG_CTRL1, ctrl1 | 0x20)?;

        // Write time registers
        self.write_reg(REG_SECONDS, dec_to_bcd(dt.seconds))?;
        self.write_reg(REG_MINUTES, dec_to_bcd(dt.minutes))?;
        self.write_reg(REG_HOURS, dec_to_bcd(dt.hours))?;
        self.write_reg(REG_DAYS, dec_to_bcd(dt.day))?;
        self.write_reg(REG_WEEKDAYS, dt.weekday)?;
        self.write_reg(REG_MONTHS, dec_to_bcd(dt.month))?;
        self.write_reg(REG_YEARS, dec_to_bcd(dt.year))?;

        // Restart oscillator
        self.write_reg(REG_CTRL1, ctrl1 & !0x20)?;
        Ok(())
    }

    /// Clear any pending timer/alarm interrupts.
    pub fn clear_interrupts(&mut self) -> Result<(), I::Error> {
        let ctrl2 = self.read_reg(REG_CTRL2)?;
        // Clear TF (bit 3) and AF (bit 6)
        let new_ctrl2 = ctrl2 & !((1 << 3) | (1 << 6));
        self.write_reg(REG_CTRL2, new_ctrl2)
    }

    /// Enable countdown timer to trigger an interrupt after `seconds`.
    pub fn enable_timer(&mut self, seconds: u32) -> Result<(), I::Error> {
        // Disable timer first
        self.write_reg(REG_TIMER_MODE, 0x00)?;

        let (tcf, val) = if seconds <= 255 {
            // 1 Hz clock
            (0b10, seconds as u8)
        } else {
            // 1/60 Hz clock
            let minutes = (seconds + 59) / 60;
            let minutes = if minutes > 255 { 255 } else { minutes };
            (0b11, minutes as u8)
        };

        // Write timer value
        self.write_reg(REG_TIMER_VAL, val)?;

        // Timer mode: TCF (bits 4:3), TE=1 (bit 2), TIE=1 (bit 1), TI_TP=0 (bit 0)
        let mode = (tcf << 3) | 0b0110;
        self.write_reg(REG_TIMER_MODE, mode)?;

        Ok(())
    }

    /// Disable the countdown timer.
    pub fn disable_timer(&mut self) -> Result<(), I::Error> {
        self.write_reg(REG_TIMER_MODE, 0x00)
    }
}

fn bcd_to_dec(bcd: u8) -> u8 {
    (bcd >> 4) * 10 + (bcd & 0x0F)
}

fn dec_to_bcd(dec: u8) -> u8 {
    ((dec / 10) << 4) | (dec % 10)
}
