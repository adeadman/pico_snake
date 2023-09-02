//! Blink the RP Pico W onboard LED
//! Does not work with the RP Pico (non-W) which uses GPIO pin 25 to drive the onboard LED

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use core::cell::RefCell;

use defmt::*;
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::spi;
use embassy_rp::spi::{Blocking, Spi};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::{Delay, Duration, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, MonoTextStyle},
    pixelcolor::Rgb565,
    prelude::*,
    text::Text,
};
use mipidsi::Builder;
//use embedded_graphics_core::draw_target::DrawTarget;
//use st7789::{Orientation, ST7789};
use {defmt_rtt as _, panic_probe as _};

use crate::waveshare_display_interface::SPIDeviceInterface;

const H: i32 = 240;
const W: i32 = 240;

const DISPLAY_FREQ: u32 = 64_000_000;
const SPI_FREQ: u32 = 200_000;


#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    info!("Hello RP2040!");

    let bl = p.PIN_13;
    let rst = p.PIN_12;
    let display_cs = p.PIN_9;
    let dcx = p.PIN_8;
    let mosi = p.PIN_11;
    // miso is not needed as there is only a master-to-slave data output for display
    let clk = p.PIN_10;

    let btn_a = Input::new(p.PIN_15, Pull::Up);
    let btn_b = Input::new(p.PIN_17, Pull::Up);
    let btn_x = Input::new(p.PIN_19, Pull::Up);
    let btn_y = Input::new(p.PIN_21, Pull::Up);

    let btn_u = Input::new(p.PIN_2, Pull::Up);
    let btn_d = Input::new(p.PIN_18, Pull::Up);
    let btn_l = Input::new(p.PIN_16, Pull::Up);
    let btn_r = Input::new(p.PIN_20, Pull::Up);
    let btn_c = Input::new(p.PIN_3, Pull::Up);

    // Set up Serial Peripheral Interface (SPI)
    let mut display_config = spi::Config::default();
    display_config.frequency = DISPLAY_FREQ;
    display_config.phase = spi::Phase::CaptureOnSecondTransition;
    display_config.polarity = spi::Polarity::IdleHigh;
    let mut spi_config = spi::Config::default();
    spi_config.frequency = SPI_FREQ;
    spi_config.phase = spi::Phase::CaptureOnSecondTransition;
    spi_config.polarity = spi::Polarity::IdleHigh;

    let spi: Spi<'_, _, Blocking> = Spi::new_blocking_txonly(p.SPI1, clk, mosi, spi_config);
    let spi_bus: Mutex<NoopRawMutex, _> = Mutex::new(RefCell::new(spi));

    let display_spi = SpiDeviceWithConfig::new(&spi_bus, Output::new(display_cs, Level::High), display_config);
    let dcx = Output::new(dcx, Level::Low);
    let rst = Output::new(rst, Level::Low);

    // Display Interface abstraction from SPI and DC
    let di = SPIDeviceInterface::new(display_spi, dcx);

    // create display driver
    //let mut display = ST7789::new(di, Some(rst), Some(bl), H, W);
    let mut display = Builder::st7789(di)
        .with_display_size(H as u16, W as u16)
        .with_orientation(mipidsi::Orientation::Landscape(true))
        .with_invert_colors(mipidsi::ColorInversion::Inverted)
        .init(&mut Delay, Some(rst))
        .unwrap();

    let style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let text = "Hello embedded_graphics \n + embassy + RP2040!";
    let mut text_x = 10;
    let mut text_y = 100;

    // Enable LCD backlight
    let _bl = Output::new(bl, Level::High);

    loop {
        if btn_u.is_low() {
            text_y -= 5;
        }
        if btn_d.is_low() {
            text_y += 5;
        }
        if btn_l.is_low() {
            text_x -= 5;
        }
        if btn_r.is_low() {
            text_x += 5;
        }

        // constrain text_x and text_y
        text_x = if text_x < 0 { 0 } else { text_x };
        text_x = if text_x > W { W } else { text_x };
        text_y = if text_y < 0 { 0 } else if text_y > H { H } else { text_y };
        // clear display
        display.clear(Rgb565::BLACK).unwrap();
        Text::new(text, Point::new(text_x, text_y), style)
            .draw(&mut display)
            .unwrap();
        // wait 100ms
        //Timer::after(Duration::from_millis(100)).await;
    }
}

mod waveshare_display_interface {
    use display_interface::{DataFormat, DisplayError, WriteOnlyDataCommand};
    use embedded_hal_1::digital::OutputPin;
    use embedded_hal_1::spi::SpiDevice;

    /// SPI display interface
    ///
    /// Combines the SPI peripheral and the data/command pin
    pub struct SPIDeviceInterface<SPI, DC> {
        spi: SPI,
        dc: DC,
    }

    impl<SPI, DC> SPIDeviceInterface<SPI, DC>
    where
        SPI: SpiDevice,
        DC: OutputPin,
    {
        pub fn new(spi: SPI, dc: DC) -> Self {
            Self { spi, dc }
        }
    }

    impl<SPI, DC> WriteOnlyDataCommand for SPIDeviceInterface<SPI, DC>
    where
        SPI: SpiDevice,
        DC: OutputPin,
    {
        fn send_commands(&mut self, cmds: DataFormat<'_>) -> Result<(), DisplayError> {
            // 1 = data, 0 = command
            self.dc.set_low().map_err(|_| DisplayError::DCError)?;

            send_u8(&mut self.spi, cmds).map_err(|_| DisplayError::BusWriteError)?;
            Ok(())
        }

        fn send_data(&mut self, buf: DataFormat<'_>) -> Result<(), DisplayError> {
            // 1 = data, 0 = command
            self.dc.set_high().map_err(|_| DisplayError::DCError)?;

            send_u8(&mut self.spi, buf).map_err(|_| DisplayError::BusWriteError)?;
            Ok(())
        }
    }

    fn send_u8<T: SpiDevice>(spi: &mut T, words: DataFormat<'_>) -> Result<(), T::Error> {
        match words {
            DataFormat::U8(slice) => spi.write(slice),
            DataFormat::U16(slice) => {
                use byte_slice_cast::*;
                spi.write(slice.as_byte_slice())
            }
            DataFormat::U16LE(slice) => {
                use byte_slice_cast::*;
                for v in slice.as_mut() {
                    *v = v.to_le();
                }
                spi.write(slice.as_byte_slice())
            }
            DataFormat::U16BE(slice) => {
                use byte_slice_cast::*;
                for v in slice.as_mut() {
                    *v = v.to_be();
                }
                spi.write(slice.as_byte_slice())
            }
            DataFormat::U8Iter(iter) => {
                let mut buf = [0; 32];
                let mut i = 0;

                for v in iter.into_iter() {
                    buf[i] = v;
                    i += 1;

                    // If we reach the maximum 32 byte, flush write and start again
                    if i == buf.len() {
                        spi.write(&buf)?;
                        i = 0;
                    }
                }

                if i > 0 {
                    spi.write(&buf[..i])?;
                }

                Ok(())
            }
            DataFormat::U16LEIter(iter) => {
                use byte_slice_cast::*;
                let mut buf = [0; 64];
                let mut i = 0;
                let len = buf.len();

                for v in iter.map(u16::to_le) {
                    buf[i] = v;
                    i += 1;

                    if i == len {
                        spi.write(&buf.as_byte_slice())?;
                        i = 0;
                    }
                }

                if i > 0 {
                    spi.write(&buf[..i].as_byte_slice())?;
                }

                Ok(())
            }
            DataFormat::U16BEIter(iter) => {
                use byte_slice_cast::*;
                let mut buf = [0; 64];
                let mut i = 0;
                let len = buf.len();

                for v in iter.map(u16::to_be) {
                    buf[i] = v;
                    i += 1;

                    if i == len {
                        spi.write(&buf.as_byte_slice())?;
                        i = 0;
                    }
                }

                if i > 0 {
                    spi.write(&buf[..i].as_byte_slice())?;
                }

                Ok(())
            }
            _ => unimplemented!(),
        }
    }
}
