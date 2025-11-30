use core::fmt::Display;

use defmt::Format;
// use allocator_api2::boxed::Box;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::{
    Async,
    gpio::{Level, interconnect::PeripheralOutput},
    peripherals::RMT,
    rmt::{Channel, PulseCode, Rmt, Tx, TxChannelConfig, TxChannelCreator},
    time::Rate,
};

pub struct NeoPixelDriver<'a> {
    channel: Channel<'a, Async, Tx>,
    led: HSV,
    last_data_sent: Instant,
    // fut: Option<Box<dyn Future<Output = Result<(), rmt::Error>>>>,
}

impl<'a> NeoPixelDriver<'a> {
    pub fn new(rmt: RMT<'a>, pin: impl PeripheralOutput<'a>) -> anyhow::Result<Self> {
        let led = HSV::default();
        let rmt = Rmt::new(rmt, Rate::from_mhz(40))?.into_async();
        let tx_config = TxChannelConfig::default()
            .with_memsize(4)
            .with_clk_divider(1)
            .with_idle_output(true)
            .with_carrier_level(Level::Low)
            .with_carrier_modulation(false);
        let channel = rmt.channel0.configure_tx(pin, tx_config)?;
        Ok(Self {
            channel,
            led,
            last_data_sent: Instant::now(),
            // fut: None,
        })
    }

    pub async fn set_led(&mut self, hsv: HSV) -> anyhow::Result<()> {
        self.led = hsv;
        self.transmit().await
    }

    async fn transmit(&mut self) -> anyhow::Result<()> {
        let elapsed = self.last_data_sent.elapsed();
        if let Some(delta) = Duration::from_micros(200).checked_sub(elapsed) {
            Timer::after(delta).await;
        }
        let codes = self.led.to_rgb().to_pulsecodes();
        self.channel.transmit(&codes).await?;
        // self.fut = Some(Box::new(fut) as dyn Future<Output = Result<_, _>>);
        self.last_data_sent = Instant::now();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RGB {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RGB {
    pub fn to_pulsecodes(&self) -> [PulseCode; 25] {
        let mut codes = [PulseCode::default(); 25];
        let (mut r, mut g, mut b) = (self.r, self.g, self.b);
        for i in 0..8 {
            codes[0 * 8 + i] = if g & 0x80 != 0 { RMT_ONE } else { RMT_ZERO };
            g <<= 1;
        }
        for i in 0..8 {
            codes[1 * 8 + i] = if r & 0x80 != 0 { RMT_ONE } else { RMT_ZERO };
            r <<= 1;
        }
        for i in 0..8 {
            codes[2 * 8 + i] = if b & 0x80 != 0 { RMT_ONE } else { RMT_ZERO };
            b <<= 1;
        }
        codes
    }
}

const RMT_ZERO: PulseCode = PulseCode::new(Level::High, 12, Level::Low, 36);
const RMT_ONE: PulseCode = PulseCode::new(Level::High, 24, Level::Low, 24);

#[derive(Debug, Clone, Copy, PartialEq, Default, Format)]
pub struct HSV {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

impl Display for HSV {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "(h: {}, s: {}, v: {})", self.h, self.s, self.v)
    }
}
//
// impl Format for HSV {
//     fn format(&self, fmt: defmt::Formatter) {
//         write!(fmt, "(h: {}, s: {}, v: {})", self.h, self.s, self.v)
//     }
// }

impl HSV {
    pub fn to_rgb(&self) -> RGB {
        let c = self.v * self.s;
        let hp = self.h / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r1, g1, b1) = if hp < 1.0 {
            (c, x, 0.0)
        } else if hp < 2.0 {
            (x, c, 0.0)
        } else if hp < 3.0 {
            (0.0, c, x)
        } else if hp < 4.0 {
            (0.0, x, c)
        } else if hp < 5.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };
        let m = self.v - c;
        let (r, g, b) = (r1 + m, g1 + m, b1 + m);

        RGB {
            r: (r * 255.0) as u8,
            g: (g * 255.0) as u8,
            b: (b * 255.0) as u8,
        }
    }
}
