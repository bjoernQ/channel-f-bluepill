#![no_std]
#![no_main]
#![feature(asm)]

use nb::block;
use panic_halt as _;
use rtt_target::rtt_init_print;

use cortex_m_rt::entry;

use spi_slave::Spi1Slave;
use stm32f1xx_hal::{
    pac::{self},
    prelude::*,
};

use embedded_hal::{
    digital::v2::OutputPin,
    spi::{Mode, Phase, Polarity},
};
use video::VID_RAM;

mod spi_slave;
mod video;

const COLORS: [u8; 16] = [
    0b000000, 0b111111, 0b111111, 0b111111, //
    0b101011, 0b000011, 0b110000, 0b001000, //
    0b010101, 0b000011, 0b110000, 0b001000, //
    0b011101, 0b000011, 0b110000, 0b001000, //
];

#[entry]
fn main() -> ! {
    let mut indexed_pixels = [0u8; 128 * 64];
    rtt_init_print!();

    // Get access to the core peripherals from the cortex-m crate
    let mut cp = cortex_m::Peripherals::take().unwrap();
    // Get access to the device specific peripherals from the peripheral access crate
    let dp = pac::Peripherals::take().unwrap();

    // Take ownership over the raw flash and rcc devices and convert them into the corresponding
    // HAL structs
    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.constrain();

    let _clocks = rcc
        .cfgr
        .use_hse(8.mhz())
        .sysclk(72.mhz())
        .pclk1(36.mhz())
        .pclk2(72.mhz())
        .freeze(&mut flash.acr);

    let mut afio = dp.AFIO.constrain(&mut rcc.apb2);
    let mut gpioa = dp.GPIOA.split(&mut rcc.apb2);
    let mut gpiob = dp.GPIOB.split(&mut rcc.apb2);
    let (_pa15, pb3, pb4) = afio.mapr.disable_jtag(gpioa.pa15, gpiob.pb3, gpiob.pb4);

    // configure video pins
    let _pb3 = pb3.into_push_pull_output(&mut gpiob.crl);
    let _pb4 = pb4.into_push_pull_output(&mut gpiob.crl);
    let _pb5 = gpiob.pb5.into_push_pull_output(&mut gpiob.crl);
    let _pb6 = gpiob.pb6.into_push_pull_output(&mut gpiob.crl);
    let _pb7 = gpiob.pb7.into_push_pull_output(&mut gpiob.crl);
    let _pb8 = gpiob.pb8.into_push_pull_output(&mut gpiob.crh);
    let _pb9 = gpiob.pb9.into_alternate_push_pull(&mut gpiob.crh); // timer controlled

    // for testing use the onboard led
    let mut gpioc = dp.GPIOC.split(&mut rcc.apb2);
    let mut led = gpioc.pc13.into_push_pull_output(&mut gpioc.crh);

    // PA0 is used to signal busy
    let busy = gpioa.pa0.into_push_pull_output(&mut gpioa.crl);

    led.set_high().unwrap_or_default(); // on board LED off

    // prepare video stuff
    video::init_video(&mut cp, dp.TIM4, dp.TIM1, busy);

    // start video output
    video::start_video();

    // fill black into the vid-ram
    for i in 0..(64 * 128) {
        unsafe {
            VID_RAM[i as usize] = 0;
        }
    }

    let pins = (
        gpioa.pa4.into_pull_down_input(&mut gpioa.crl), // NSS
        gpioa.pa5.into_alternate_push_pull(&mut gpioa.crl), // SCK
        gpioa.pa6.into_alternate_push_pull(&mut gpioa.crl), // MISO
        gpioa.pa7.into_floating_input(&mut gpioa.crl),  // MOSI
    );

    let spi_mode = Mode {
        polarity: Polarity::IdleLow,
        phase: Phase::CaptureOnFirstTransition,
    };
    let mut spi = Spi1Slave::spi1slave(dp.SPI1, pins, spi_mode, &mut rcc.apb2);

    spi.send(0x7f).unwrap();
    spi.clear_ovr();

    let mut d;

    // XXXXXXXY YYYYYCCC

    let mut z = 0u8;
    let mut last_byte = 0u8;
    let mut was_ovr = false;

    let mut cnt = 0;

    loop {
        cnt += 1;
        if cnt >= 5000 {
            cnt = 0;
            led.toggle().unwrap();
        }

        let snd_byte = if !was_ovr { 0x7f } else { 0xff };

        let res = block!(spi.send(snd_byte));
        if let Err(e) = res {
            match e {
                stm32f1xx_hal::spi::Error::Overrun => {
                    spi.clear_ovr();
                    continue;
                }
                stm32f1xx_hal::spi::Error::ModeFault => {}
                stm32f1xx_hal::spi::Error::Crc => {}
                stm32f1xx_hal::spi::Error::_Extensible => {}
            }
        }

        let res = block!(spi.read());

        d = match res {
            Ok(v) => {
                was_ovr = false;
                v
            }
            Err(e) => {
                was_ovr = true;
                match e {
                    stm32f1xx_hal::spi::Error::Overrun => {
                        spi.clear_ovr();
                    }
                    stm32f1xx_hal::spi::Error::ModeFault => {}
                    stm32f1xx_hal::spi::Error::Crc => {}
                    stm32f1xx_hal::spi::Error::_Extensible => {}
                }
                255
            }
        };

        if z == 1 && !was_ovr {
            let x = (last_byte & 0b11111110) >> 1;
            let y = ((last_byte & 1) << 5) | ((d & 0b11111000) >> 3);
            let mut c = d & 0b111;

            if x == 125 || x == 126 {
                handle_palette_change(y, x, c, &mut indexed_pixels);
            } else {
                let palette = unsafe {
                    ((VID_RAM[y as usize * 128usize + 125usize] & 2 >> 1)
                        | (VID_RAM[y as usize * 128usize + 126usize]))
                        & 0b11
                } as usize;
                c = COLORS[c as usize + palette * 4usize];

                let offset = y as usize * 128usize + x as usize;
                indexed_pixels[offset] = d & 0b111;
                unsafe {
                    VID_RAM[offset] = c;
                }    
            }

            z = 0;
        } else if !was_ovr && z == 0 {
            last_byte = d;
            z = z + 1;
        }
    }
}

fn handle_palette_change(y: u8, x: u8, new_palette_value: u8, indexed_pixels: &mut [u8]) {
    let current_palette = unsafe {
        ((VID_RAM[y as usize * 128usize + 125usize]  & 2 >> 1)
            | (VID_RAM[y as usize * 128usize + 126usize]))
            & 0b11
    } as usize;

    let new_palette = if x == 125 {
        unsafe {
            ((new_palette_value  & 2 >> 1) | (VID_RAM[y as usize * 128usize + 126usize])) & 0b11
        }
    } else {
        unsafe {
            ((VID_RAM[y as usize * 128usize + 125usize]  & 2 >> 1) | (new_palette_value)) & 0b11
        }
    } as usize;

    if current_palette == new_palette {
        return;
    }

    for xx in 0..102 {
        unsafe {
            VID_RAM[y as usize * 128usize + xx] =
                COLORS[new_palette * 4 + indexed_pixels[xx + y as usize * 128usize] as usize];
        }
    }

    let offset = y as usize * 128usize + x as usize;
    indexed_pixels[offset] = new_palette_value;
    unsafe {
        VID_RAM[offset] = new_palette_value;
    }
}
