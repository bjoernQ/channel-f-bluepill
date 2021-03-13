#![no_std]
#![no_main]
#![feature(asm)]
#![feature(llvm_asm)]
#![feature(fmt_internals)]

use core::cell::RefCell;

use chf_emulator::{ChannelF, Cpu};
use embedded_sdmmc::{SdMmcSpi, TimeSource, VolumeIdx};
use nb::block;
use panic_halt as _;
use rtt_target::{rprintln, rtt_init_print};

use cortex_m_rt::entry;

use stm32f1xx_hal::{
    delay::Delay,
    gpio::{
        gpioa::PA9,
        gpiob::{PB13, PB14, PB15},
        gpioc::PC13,
        Alternate, Floating, Input, Output, PullDown, PushPull,
    },
    pac::{self, SPI2},
    prelude::*,
    pwm::Channel,
    spi::{Spi, Spi2NoRemap},
    timer::{Tim2NoRemap, Timer},
};

use embedded_hal::{
    digital::v2::{InputPin, OutputPin},
    spi::{Mode, Phase, Polarity},
};

#[entry]
fn main() -> ! {
    rtt_init_print!();

    // Get access to the core peripherals from the cortex-m crate
    let cp = cortex_m::Peripherals::take().unwrap();
    // Get access to the device specific peripherals from the peripheral access crate
    let dp = pac::Peripherals::take().unwrap();

    // Take ownership over the raw flash and rcc devices and convert them into the corresponding
    // HAL structs
    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.constrain();

    let clocks = rcc
        .cfgr
        .use_hse(8.mhz())
        .sysclk(72.mhz())
        .pclk1(36.mhz())
        .pclk2(72.mhz())
        .freeze(&mut flash.acr);

    let mut afio = dp.AFIO.constrain(&mut rcc.apb2);
    let mut gpioa = dp.GPIOA.split(&mut rcc.apb2);
    let mut gpiob = dp.GPIOB.split(&mut rcc.apb2);
    let (pa15, pb3, pb4) = afio.mapr.disable_jtag(gpioa.pa15, gpiob.pb3, gpiob.pb4);

    // for testing use the onboard led
    let mut gpioc = dp.GPIOC.split(&mut rcc.apb2);
    let mut led = gpioc.pc13.into_push_pull_output(&mut gpioc.crh);

    led.set_high().unwrap_or_default(); // on board LED off

    let button_1 = gpiob.pb9.into_pull_down_input(&mut gpiob.crh);
    let button_2 = gpiob.pb8.into_pull_down_input(&mut gpiob.crh);
    let button_3 = gpiob.pb7.into_pull_down_input(&mut gpiob.crl);
    let button_4 = gpiob.pb6.into_pull_down_input(&mut gpiob.crl);

    let mut select_controller_0 = gpioa.pa1.into_push_pull_output(&mut gpioa.crl);
    let mut select_controller_1 = gpioa.pa2.into_push_pull_output(&mut gpioa.crl);
    let left = gpioa.pa3.into_pull_down_input(&mut gpioa.crl);
    let right = gpioa.pa4.into_pull_down_input(&mut gpioa.crl);
    let up = gpioa.pa5.into_pull_down_input(&mut gpioa.crl);
    let down = gpioa.pa6.into_pull_down_input(&mut gpioa.crl);
    let ccw = gpioa.pa7.into_pull_down_input(&mut gpioa.crl);
    let cw = gpiob.pb0.into_pull_down_input(&mut gpiob.crl);
    let pull = gpiob.pb1.into_pull_down_input(&mut gpiob.crl);
    let push = gpiob.pb10.into_pull_down_input(&mut gpiob.crh);

    let peer_bsy = gpioa.pa9.into_pull_down_input(&mut gpioa.crh);

    let mut delay = Delay::new(cp.SYST, clocks);

    // TIM2
    let c1 = gpioa.pa0.into_alternate_push_pull(&mut gpioa.crl);
    // If you don't want to use all channels, just leave some out
    // let c4 = gpioa.pa3.into_alternate_push_pull(&mut gpioa.crl);
    let pins = c1;

    let mut pwm = Timer::tim2(dp.TIM2, &clocks, &mut rcc.apb1).pwm::<Tim2NoRemap, _, _, _>(
        pins,
        &mut afio.mapr,
        120.hz(),
    );

    let max = pwm.get_max_duty();
    pwm.set_duty(Channel::C1, max / 2);
    pwm.disable(Channel::C1);

    // SPI2: other MCU
    let pins = (
        gpiob.pb13.into_alternate_push_pull(&mut gpiob.crh), // SCK
        gpiob.pb14.into_floating_input(&mut gpiob.crh),      // MISO
        gpiob.pb15.into_alternate_push_pull(&mut gpiob.crh), // MOSI
    );
    let spi_mode = Mode {
        polarity: Polarity::IdleLow,
        phase: Phase::CaptureOnFirstTransition,
    };
    let mut spi = Spi::spi2(dp.SPI2, pins, spi_mode, 20.mhz(), clocks, &mut rcc.apb1);

    let pins_sd = (
        pb3.into_alternate_push_pull(&mut gpiob.crl),
        pb4,
        gpiob.pb5.into_alternate_push_pull(&mut gpiob.crl), // MOSI
    );
    let spi_mode = Mode {
        polarity: Polarity::IdleLow,
        phase: Phase::CaptureOnFirstTransition,
    };
    let spi_sd = Spi::spi1(
        dp.SPI1,
        pins_sd,
        &mut afio.mapr,
        spi_mode,
        300.khz(),
        clocks,
        &mut rcc.apb2,
    );

    let pa15 = pa15.into_push_pull_output(&mut gpioa.crh);
    let sd = SdMmcSpi::new(spi_sd, pa15);

    let time_source = FakeTimeSource {};
    let mut controller = embedded_sdmmc::Controller::new(sd, time_source);
    controller.device().init().unwrap();

    let volume = controller.get_volume(VolumeIdx(0));
    let mut volume = volume.unwrap();
    let dir = controller.open_root_dir(&volume);
    let dir = dir.unwrap();

    let mut files = [0u8; 8 * 128]; // 64 files supported at max
    let mut idx = 0;
    let mut file_count = 0;

    controller
        .iterate_dir(&volume, &dir, |entry| {
            struct WriteBuffer {
                buffer: [u8; 12],
                idx: usize,
            }

            impl core::fmt::Write for WriteBuffer {
                fn write_str(&mut self, s: &str) -> core::fmt::Result {
                    for c in s.chars() {
                        self.buffer[self.idx] = c as u8;
                        self.idx += 1;
                    }
                    Ok(())
                }
            }

            let mut write_buffer = WriteBuffer {
                buffer: [0u8; 12],
                idx: 0usize,
            };

            let fname = entry.name.clone();
            core::fmt::write(&mut write_buffer, format_args!("{}", fname)).unwrap_or_default();

            if write_buffer.buffer[9] == b'B' {
                for i in 0..8 {
                    files[idx + i] = write_buffer.buffer[i];
                }

                idx += 8;
                file_count += 1;
            }
        })
        .unwrap();

    let mut pb12 = gpiob.pb12.into_push_pull_output(&mut gpiob.crh);
    pb12.set_high().unwrap();

    let mut x: u8;
    let mut y: u8;
    let mut c: u8;

    let mut playing_sound = chf_emulator::Sound::Silence;

    let channel_f = StmChannelF {
        should_set_pixel: RefCell::from(false),
        x: RefCell::from(0),
        y: RefCell::from(0),
        color: RefCell::from(0),

        current_sound: RefCell::from(chf_emulator::Sound::Silence),

        key_1: RefCell::new(false),
        key_2: RefCell::new(false),
        key_3: RefCell::new(false),
        key_4: RefCell::new(false),
        r0: RefCell::new(false),
        l0: RefCell::new(false),
        u0: RefCell::new(false),
        d0: RefCell::new(false),
        ccw0: RefCell::new(false),
        cw0: RefCell::new(false),
        pull0: RefCell::new(false),
        push0: RefCell::new(false),
        r1: RefCell::new(false),
        l1: RefCell::new(false),
        u1: RefCell::new(false),
        d1: RefCell::new(false),
        ccw1: RefCell::new(false),
        cw1: RefCell::new(false),
        pull1: RefCell::new(false),
        push1: RefCell::new(false),
    };

    pb12.set_low().unwrap(); // keep NSS low all the time

    // clear screen
    for y in 0..64 {
        for x in 0..127 {
            set_pixel(x, y, 0, &peer_bsy, &mut spi, &mut delay, &mut led);
            delay.delay_us(85u16);
        }
    }

    select_controller_0.set_high().unwrap_or_default();
    delay.delay_us(300u16);

    let mut fileindex = 0;
    let mut shown_fileindex = 999;
    let mut left_pressed = false;
    let mut right_pressed = false;
    let mut push_pressed = false;
    loop {
        if shown_fileindex != fileindex {
            let idx = fileindex * 8;
            let fname = core::str::from_utf8(&files[idx..(idx + 8)]).unwrap_or_default();
            draw_str(10, 10, fname, &peer_bsy, &mut spi, &mut delay, &mut led);
            shown_fileindex = fileindex;
        }

        if left_pressed && left.is_low().unwrap_or_default() && fileindex > 0 {
            fileindex -= 1;
        }

        if right_pressed && right.is_low().unwrap_or_default() && fileindex < file_count {
            fileindex += 1;
        }

        if push_pressed && push.is_low().unwrap_or_default() {
            break;
        }

        left_pressed = left.is_high().unwrap_or_default();
        right_pressed = right.is_high().unwrap_or_default();
        push_pressed = push.is_high().unwrap_or_default();

        delay.delay_us(300u16);
    }

    select_controller_0.set_low().unwrap_or_default();

    // load the cartridge
    let idx = fileindex * 8;
    let mut full_fname = [0u8; 12];
    for i in 0..8 {
        full_fname[i] = files[idx + i];
    }
    full_fname[8] = b'.';
    full_fname[9] = b'B';
    full_fname[10] = b'I';
    full_fname[11] = b'N';

    let file_to_load = core::str::from_utf8(&full_fname).unwrap_or_default();

    let mut file = controller
        .open_file_in_dir(
            &mut volume,
            &dir,
            file_to_load,
            embedded_sdmmc::Mode::ReadOnly,
        )
        .unwrap();
    unsafe {
        controller
            .read(&volume, &mut file, &mut CARTRIDGE)
            .unwrap_or_default();
    }

    let catridge = unsafe { CARTRIDGE };
    let mut cpu = Cpu::new(ROM_0000, ROM_0400, &catridge, &channel_f);
    cpu.reset();

    let mut cnt = 0;
    loop {
        cnt += 1;
        if cnt > 10000 {
            cnt = 0;

            // checking keys is quite slow - better use complete reads of the GPIO registers
            handle_keys(
                &mut select_controller_0,
                &mut select_controller_1,
                &button_1,
                &button_2,
                &button_3,
                &button_4,
                &right,
                &left,
                &up,
                &down,
                &ccw,
                &cw,
                &pull,
                &push,
                &channel_f,
            )
        }

        cpu.cycles = 0;
        let opcode = cpu.fetch();
        cpu.execute(opcode);

        let should_set_pixel = channel_f.should_set_pixel.replace(false);

        if should_set_pixel {
            x = channel_f.x.take();
            y = channel_f.y.take();
            c = channel_f.color.take();
            set_pixel(x, y, c, &peer_bsy, &mut spi, &mut delay, &mut led);
        }

        if playing_sound != *(channel_f.current_sound.borrow()) {
            playing_sound = *(channel_f.current_sound.borrow());

            match playing_sound {
                chf_emulator::Sound::Silence => {
                    pwm.disable(Channel::C1);
                }
                chf_emulator::Sound::Frequency1Khz => {
                    pwm.set_period(1000.hz());
                    pwm.enable(Channel::C1);
                }
                chf_emulator::Sound::Frequency500Hz => {
                    pwm.set_period(500.hz());
                    pwm.enable(Channel::C1);
                }
                chf_emulator::Sound::Frequency120Hz => {
                    pwm.set_period(120.hz());
                    pwm.enable(Channel::C1);
                }
            }
        }
    }
}

fn draw_str(
    x: u8,
    y: u8,
    value: &str,
    peer_bsy: &PA9<Input<PullDown>>,
    spi: &mut Spi<
        SPI2,
        Spi2NoRemap,
        (
            PB13<Alternate<PushPull>>,
            PB14<Input<Floating>>,
            PB15<Alternate<PushPull>>,
        ),
    >,
    delay: &mut Delay,
    led: &mut PC13<Output<PushPull>>,
) {
    let mut xx = x;
    for c in value.chars() {
        for i in (0..CHARACTERS.len()).step_by(9) {
            if CHARACTERS[i] == c as u8 {
                let data = &CHARACTERS[i + 1..i + 9];
                draw_character(xx, y, data, peer_bsy, spi, delay, led);
            }
        }
        xx += 8;
    }
}

fn draw_character(
    x: u8,
    y: u8,
    data: &[u8],
    peer_bsy: &PA9<Input<PullDown>>,
    spi: &mut Spi<
        SPI2,
        Spi2NoRemap,
        (
            PB13<Alternate<PushPull>>,
            PB14<Input<Floating>>,
            PB15<Alternate<PushPull>>,
        ),
    >,
    delay: &mut Delay,
    led: &mut PC13<Output<PushPull>>,
) {
    for yy in 0..8 {
        for xx in 0u8..8u8 {
            let color = data[yy].overflowing_shr((8 - xx) as u32).0 & 1;
            set_pixel(x + xx, y + yy as u8, color, peer_bsy, spi, delay, led);
            delay.delay_us(85u16);
        }
    }
}

fn set_pixel(
    x: u8,
    y: u8,
    c: u8,
    peer_bsy: &PA9<Input<PullDown>>,
    spi: &mut Spi<
        SPI2,
        Spi2NoRemap,
        (
            PB13<Alternate<PushPull>>,
            PB14<Input<Floating>>,
            PB15<Alternate<PushPull>>,
        ),
    >,
    delay: &mut Delay,
    led: &mut PC13<Output<PushPull>>,
) {
    let mut was_err = false;
    let mut snd_byte = 0u8;
    let mut z: u8 = 0;

    // XXXXXXXY YYYYYCCC + 0x00

    // if it's not a pixel in the safe are and not a "select palette" pixel ... don't transmit it
    if (x < 4 || x > 101 || y < 4 || y > 62) && x != 125 && x != 126 {
        return;
    }

    // set pixel stuff ... quite ugly right now
    loop {
        if z == 0 {
            snd_byte = x.overflowing_shl(1).0 | (y.overflowing_shr(5).0 & 1);
        } else if z == 1 {
            snd_byte = y.overflowing_shl(3).0 | c;
        }

        let res = block!(spi.send(snd_byte));

        if let Err(_) = res {
            // can we do anything about it here and now?
        }

        // for some reason things get out of sync w/o a delay
        // this is because the slave is not able to read and send via SPI
        // during a scanline
        // by looking at the busy pin we know if the other MCU
        // is currently drawing pixels or not
        let additional_wait_time = if x == 125 || x == 126 {
            10 // will re-color
        } else {
            0
        };

        let wait_time = if peer_bsy.is_high().unwrap_or_default() {
            65u16
        } else {
            15u16
        } + additional_wait_time;
        delay.delay_us(wait_time);

        let res = block!(spi.read());
        match res {
            Ok(_v) => {
                if _v != 0x7f {
                    led.toggle().unwrap();
                    was_err = true;
                }
            }
            Err(e) => {
                rprintln!("send err {:?}", e);
                was_err = true;
            }
        }

        if !was_err {
            z += 1;

            if z >= 2 {
                break;
            }
        }

        was_err = false;
    }
}

#[derive(Debug)]
pub struct StmChannelF {
    should_set_pixel: RefCell<bool>,
    x: RefCell<u8>,
    y: RefCell<u8>,
    color: RefCell<u8>,

    current_sound: RefCell<chf_emulator::Sound>,

    key_1: RefCell<bool>,
    key_2: RefCell<bool>,
    key_3: RefCell<bool>,
    key_4: RefCell<bool>,

    r0: RefCell<bool>,
    l0: RefCell<bool>,
    u0: RefCell<bool>,
    d0: RefCell<bool>,
    ccw0: RefCell<bool>,
    cw0: RefCell<bool>,
    pull0: RefCell<bool>,
    push0: RefCell<bool>,

    r1: RefCell<bool>,
    l1: RefCell<bool>,
    u1: RefCell<bool>,
    d1: RefCell<bool>,
    ccw1: RefCell<bool>,
    cw1: RefCell<bool>,
    pull1: RefCell<bool>,
    push1: RefCell<bool>,
}

impl ChannelF for StmChannelF {
    fn sound(&self, frequency: chf_emulator::Sound) {
        self.current_sound.replace(frequency);
    }

    fn set_pixel(&self, x: u8, y: u8, value: u8) {
        *self.should_set_pixel.borrow_mut() = true;
        *self.x.borrow_mut() = x;
        *self.y.borrow_mut() = y;
        *self.color.borrow_mut() = value;
    }

    fn key_pressed(&self, key: chf_emulator::Key) -> bool {
        match key {
            chf_emulator::Key::Start => *self.key_1.borrow(),
            chf_emulator::Key::Hold => *self.key_2.borrow(),
            chf_emulator::Key::Mode => *self.key_3.borrow(),
            chf_emulator::Key::Time => *self.key_4.borrow(),

            chf_emulator::Key::Right0 => *self.r0.borrow(),
            chf_emulator::Key::Left0 => *self.l0.borrow(),
            chf_emulator::Key::Forward0 => *self.u0.borrow(),
            chf_emulator::Key::Back0 => *self.d0.borrow(),
            chf_emulator::Key::CounterClockwise0 => *self.ccw0.borrow(),
            chf_emulator::Key::Clockwise0 => *self.cw0.borrow(),
            chf_emulator::Key::Pull0 => *self.pull0.borrow(),
            chf_emulator::Key::Push0 => *self.push0.borrow(),

            chf_emulator::Key::Right1 => *self.r1.borrow(),
            chf_emulator::Key::Left1 => *self.l1.borrow(),
            chf_emulator::Key::Forward1 => *self.u1.borrow(),
            chf_emulator::Key::Back1 => *self.d1.borrow(),
            chf_emulator::Key::CounterClockwise1 => *self.ccw1.borrow(),
            chf_emulator::Key::Clockwise1 => *self.cw1.borrow(),
            chf_emulator::Key::Pull1 => *self.pull1.borrow(),
            chf_emulator::Key::Push1 => *self.push1.borrow(),

            _ => false,
        }
    }
}

pub fn handle_keys<O0, O1, I0, I1, I2, I3, I4, I5, I6, I7, I8, I9, I10, I11>(
    select_0: &mut O0,
    select_1: &mut O1,

    button_1: &I0,
    button_2: &I1,
    button_3: &I2,
    button_4: &I3,

    right: &I4,
    left: &I5,
    up: &I6,
    down: &I7,
    ccw: &I8,
    cw: &I9,
    pull: &I10,
    push: &I11,

    channel_f: &StmChannelF,
) where
    O0: OutputPin,
    O1: OutputPin,
    I0: InputPin,
    I1: InputPin,
    I2: InputPin,
    I3: InputPin,
    I4: InputPin,
    I5: InputPin,
    I6: InputPin,
    I7: InputPin,
    I8: InputPin,
    I9: InputPin,
    I10: InputPin,
    I11: InputPin,
{
    select_0.set_high().unwrap_or_default();
    select_1.set_low().unwrap_or_default();

    // fast switching of the select line induces wrong readings with longer wires (interferences?)
    // waiting here is not really a good thing but it solves the problem for now
    for _ in 0..200 {
        unsafe {
            asm!("nop");
        }
    }

    *channel_f.key_1.borrow_mut() = button_1.is_high().unwrap_or(false);
    *channel_f.key_2.borrow_mut() = button_2.is_high().unwrap_or(false);

    *channel_f.l0.borrow_mut() = left.is_high().unwrap_or(false);
    *channel_f.r0.borrow_mut() = right.is_high().unwrap_or(false);
    *channel_f.u0.borrow_mut() = up.is_high().unwrap_or(false);
    *channel_f.d0.borrow_mut() = down.is_high().unwrap_or(false);
    *channel_f.ccw0.borrow_mut() = ccw.is_high().unwrap_or(false);
    *channel_f.cw0.borrow_mut() = cw.is_high().unwrap_or(false);
    *channel_f.push0.borrow_mut() = push.is_high().unwrap_or(false);
    *channel_f.pull0.borrow_mut() = pull.is_high().unwrap_or(false);

    select_0.set_low().unwrap_or_default();
    select_1.set_high().unwrap_or_default();

    for _ in 0..200 {
        unsafe {
            asm!("nop");
        }
    }

    *channel_f.key_3.borrow_mut() = button_3.is_high().unwrap_or(false);
    *channel_f.key_4.borrow_mut() = button_4.is_high().unwrap_or(false);

    *channel_f.l1.borrow_mut() = left.is_high().unwrap_or(false);
    *channel_f.r1.borrow_mut() = right.is_high().unwrap_or(false);
    *channel_f.u1.borrow_mut() = up.is_high().unwrap_or(false);
    *channel_f.d1.borrow_mut() = down.is_high().unwrap_or(false);
    *channel_f.ccw1.borrow_mut() = ccw.is_high().unwrap_or(false);
    *channel_f.cw1.borrow_mut() = cw.is_high().unwrap_or(false);
    *channel_f.push1.borrow_mut() = push.is_high().unwrap_or(false);
    *channel_f.pull1.borrow_mut() = pull.is_high().unwrap_or(false);

    select_0.set_low().unwrap_or_default();
    select_1.set_low().unwrap_or_default();
}

struct FakeTimeSource;

impl TimeSource for FakeTimeSource {
    fn get_timestamp(&self) -> embedded_sdmmc::Timestamp {
        embedded_sdmmc::Timestamp {
            year_since_1970: 0,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

const ROM_0000: &'static [u8] = include_bytes!("../../chf-emulator/roms/SL31253.bin");
const ROM_0400: &'static [u8] = include_bytes!("../../chf-emulator/roms/SL31254.bin");

static mut CARTRIDGE: [u8; 4096] = [0u8; 4096];

const CHARACTERS: &'static [u8] = &[
    b'A', 0b00011000, 0b00100100, 0b01000010, 0b01000010, 0b01111110, 0b01000010, 0b01000010,
    0b01000010, b'B', 0b11111000, 0b10000100, 0b10000100, 0b11111100, 0b10000010, 0b10000010,
    0b10000010, 0b11111100, b'C', 0b11111110, 0b10000000, 0b10000000, 0b10000000, 0b10000000,
    0b10000000, 0b10000000, 0b11111110, b'D', 0b11111000, 0b10000100, 0b10000010, 0b10000010,
    0b10000010, 0b10000010, 0b10000100, 0b11111000, b'E', 0b11111110, 0b10000000, 0b10000000,
    0b11111110, 0b10000000, 0b10000000, 0b10000000, 0b11111110, b'F', 0b11111110, 0b10000000,
    0b10000000, 0b11111110, 0b10000000, 0b10000000, 0b10000000, 0b10000000, b'G', 0b11111110,
    0b10000000, 0b10000000, 0b10001110, 0b10000010, 0b10000010, 0b10000010, 0b11111110, b'H',
    0b10000010, 0b10000010, 0b10000010, 0b11111110, 0b10000010, 0b10000010, 0b10000010, 0b10000010,
    b'I', 0b00111000, 0b00010000, 0b00010000, 0b00010000, 0b00010000, 0b00010000, 0b00010000,
    0b00111000, b'J', 0b00000100, 0b00000100, 0b00000100, 0b00000100, 0b00000100, 0b00000100,
    0b01000100, 0b00111000, b'K', 0b10000010, 0b10000100, 0b10001000, 0b11110000, 0b10001000,
    0b10000100, 0b10000010, 0b10000010, b'L', 0b10000000, 0b10000000, 0b10000000, 0b10000000,
    0b10000000, 0b10000000, 0b10000000, 0b11111110, b'M', 0b10000010, 0b11000110, 0b10011010,
    0b10000010, 0b10000010, 0b10000010, 0b10000010, 0b10000010, b'N', 0b10000010, 0b11000010,
    0b10100010, 0b10010010, 0b10010010, 0b10001010, 0b10000110, 0b10000010, b'O', 0b01111100,
    0b10000010, 0b10000010, 0b10000010, 0b10000010, 0b10000010, 0b10000010, 0b01111100, b'P',
    0b11111110, 0b10000010, 0b10000010, 0b11111100, 0b10000000, 0b10000000, 0b10000000, 0b10000000,
    b'Q', 0b01111100, 0b10000010, 0b10000010, 0b10000010, 0b10000010, 0b10001010, 0b10000110,
    0b01111110, b'R', 0b11111110, 0b10000010, 0b10000010, 0b11111100, 0b10000010, 0b10000010,
    0b10000010, 0b10000010, b'S', 0b01111100, 0b10000010, 0b10000000, 0b10000000, 0b01111100,
    0b00000010, 0b10000010, 0b01111000, b'T', 0b11111110, 0b00010000, 0b00010000, 0b00010000,
    0b00010000, 0b00010000, 0b00010000, 0b00010000, b'U', 0b10000010, 0b10000010, 0b10000010,
    0b10000010, 0b10000010, 0b10000010, 0b10000010, 0b01111100, b'V', 0b10000010, 0b10000010,
    0b10000010, 0b01000100, 0b01000100, 0b01000100, 0b00101000, 0b00010000, b'W', 0b10000010,
    0b10000010, 0b10000010, 0b10000010, 0b10010010, 0b10101010, 0b11000110, 0b10000010, b'X',
    0b10000010, 0b10000010, 0b01000100, 0b01000100, 0b00011000, 0b00100100, 0b00100100, 0b10000010,
    b'Y', 0b10000010, 0b10000010, 0b01000100, 0b00101000, 0b00010000, 0b00010000, 0b00010000,
    0b00010000, b'Z', 0b11111110, 0b00000100, 0b00000100, 0b00010000, 0b00100000, 0b01000000,
    0b01000000, 0b11111110, b'0', 0b01111100, 0b10000010, 0b10000010, 0b10000010, 0b10000010,
    0b10000010, 0b10000010, 0b01111100, b'1', 0b00010000, 0b00110000, 0b01010000, 0b00010000,
    0b00010000, 0b00010000, 0b00010000, 0b00010000, b'2', 0b11111110, 0b00000010, 0b00000010,
    0b00000010, 0b11111110, 0b10000000, 0b10000000, 0b11111110, b'3', 0b11111100, 0b00000010,
    0b00000010, 0b11111100, 0b00000010, 0b00000010, 0b00000010, 0b11111100, b'4', 0b10000010,
    0b10000010, 0b10000010, 0b11111110, 0b00000010, 0b00000010, 0b00000010, 0b00000010, b'5',
    0b11111110, 0b10000000, 0b10000000, 0b10000000, 0b11111110, 0b00000010, 0b00000010, 0b11111110,
    b'6', 0b01111110, 0b10000000, 0b10000000, 0b11111100, 0b10000010, 0b10000010, 0b10000010,
    0b11111100, b'7', 0b11111110, 0b00000010, 0b00000100, 0b00001000, 0b00010000, 0b00010000,
    0b00010000, 0b00010000, b'8', 0b01111100, 0b10000010, 0b10000010, 0b01111100, 0b10000010,
    0b10000010, 0b10000010, 0b01111100, b'9', 0b11111110, 0b10000010, 0b10000010, 0b11111110,
    0b00000010, 0b00000010, 0b00000010, 0b00000010, 0b11111110,
];
