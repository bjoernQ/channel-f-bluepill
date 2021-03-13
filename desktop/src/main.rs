use std::{cell::RefCell, env, fs};

use minifb::{Key, Scale, ScaleMode, Window, WindowOptions};

use chf_emulator::Cpu;

const WIDTH: usize = 128 * 2;
const HEIGHT: usize = 64 * 2;

const ROM_0000: &'static [u8] = include_bytes!("../../chf-emulator/roms/SL31253.bin");
const ROM_0400: &'static [u8] = include_bytes!("../../chf-emulator/roms/SL31254.bin");

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut cartridge = [0u8; 1024 * 8];

    if args.len() > 1 {
        let data = fs::read(&args[1]).unwrap();
        for (i, b) in data.iter().enumerate() {
            cartridge[i] = *b;
        }
    }

    let mut seen_opcodes = [false; 256];

    let (_stream, stream_handle) = rodio::OutputStream::try_default().unwrap();

    let mut current_sound = Sound::Silence;
    let mut sink = rodio::Sink::try_new(&stream_handle).unwrap();

    let channel_f = DesktopChannelF {
        sound: RefCell::new(Sound::Silence),

        pixels: RefCell::new([0u8; 128 * 64]),

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

    let cartridge = &cartridge;
    let mut cpu = Cpu::new(ROM_0000, ROM_0400, cartridge, &channel_f);
    cpu.reset();

    let mut buffer: Vec<u32> = vec![0; WIDTH * HEIGHT];

    let mut window = Window::new(
        "Channel F - ESC to exit",
        WIDTH,
        HEIGHT,
        WindowOptions {
            resize: true,
            scale: Scale::X2,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .expect("Unable to Open Window");

    // Limit to max ~60 fps update rate
    // window.limit_update_rate(Some(std::time::Duration::from_micros(16600)));

    window.set_background_color(0, 0, 20);

    let mut cntr = 0u32;
    let mut exe_cntr = 0u32;

    let mut p_is_down = false;
    let mut o_is_down = false;
    let mut l_is_down = false;

    let mut show_info = false;
    let mut pc_low = u16::MAX;
    let mut pc_high = u16::MIN;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        *channel_f.key_1.borrow_mut() = window.is_key_down(Key::Key1);
        *channel_f.key_2.borrow_mut() = window.is_key_down(Key::Key2);
        *channel_f.key_3.borrow_mut() = window.is_key_down(Key::Key3);
        *channel_f.key_4.borrow_mut() = window.is_key_down(Key::Key4);

        *channel_f.l0.borrow_mut() = window.is_key_down(Key::A);
        *channel_f.r0.borrow_mut() = window.is_key_down(Key::D);
        *channel_f.u0.borrow_mut() = window.is_key_down(Key::W);
        *channel_f.d0.borrow_mut() = window.is_key_down(Key::S);
        *channel_f.ccw0.borrow_mut() = window.is_key_down(Key::Q);
        *channel_f.cw0.borrow_mut() = window.is_key_down(Key::E);
        *channel_f.pull0.borrow_mut() = window.is_key_down(Key::Y);
        *channel_f.push0.borrow_mut() = window.is_key_down(Key::Z);

        *channel_f.l1.borrow_mut() = window.is_key_down(Key::NumPad4);
        *channel_f.r1.borrow_mut() = window.is_key_down(Key::NumPad6);
        *channel_f.u1.borrow_mut() = window.is_key_down(Key::NumPad8);
        *channel_f.d1.borrow_mut() = window.is_key_down(Key::NumPad5);
        *channel_f.ccw1.borrow_mut() = window.is_key_down(Key::NumPad7);
        *channel_f.cw1.borrow_mut() = window.is_key_down(Key::NumPad9);
        *channel_f.pull1.borrow_mut() = window.is_key_down(Key::NumPad1);
        *channel_f.push1.borrow_mut() = window.is_key_down(Key::NumPad2);

        if window.is_key_down(Key::L) {
            l_is_down = true;
        }

        if window.is_key_released(Key::L) && l_is_down {
            l_is_down = false;
            show_info = !show_info;
        }

        if window.is_key_down(Key::O) {
            o_is_down = true;
        }

        if window.is_key_released(Key::O) && o_is_down {
            o_is_down = false;
            for i in 0..=255 {
                seen_opcodes[i] = false;
            }
        }

        if window.is_key_down(Key::P) {
            p_is_down = true;
        }

        if window.is_key_released(Key::P) && p_is_down {
            p_is_down = false;
            for i in 0..=255 {
                println!("{:x} {}", i, seen_opcodes[i]);
            }
            println!();
        }

        exe_cntr += 1;
        if exe_cntr >= 2 {
            exe_cntr = 0;

            let pc = cpu.pc0;

            cpu.cycles = 0;
            let opcode = cpu.fetch();

            seen_opcodes[opcode as usize] = true;

            if show_info {
                if pc >= pc_high {
                    pc_high = pc;
                }

                if pc <= pc_low {
                    pc_low = pc;
                }
                println!("{:x} {:x} .... {:x} - {:x}", pc, opcode, pc_low, pc_high);
            }

            cpu.execute(opcode);
        }

        if current_sound != *channel_f.sound.borrow() {
            current_sound = *channel_f.sound.borrow();
            sink.stop();
            sink = rodio::Sink::try_new(&stream_handle).unwrap();

            match current_sound {
                Sound::Silence => {}
                Sound::Frequency1Khz => {
                    let sound = SineWave::new(1000);
                    sink.append(sound);
                }
                Sound::Frequency500Hz => {
                    let sound = SineWave::new(500);
                    sink.append(sound);
                }
                Sound::Frequency120Hz => {
                    let sound = SineWave::new(120);
                    sink.append(sound);
                }
            }
        }

        cntr += 1;
        if cntr >= 42000 {
            cntr = 0;

            let pixels = channel_f.pixels.borrow();
            // column 125 + 126 choose the palette to use
            const COLORS: [u32; 16] = [
                0xff000000, 0xffffffff, 0xffffffff, 0xffffffff, //
                0xff7777ff, 0xff0000ff, 0xffff0000, 0xff008800, //
                0xffcccccc, 0xff0000ff, 0xffff0000, 0xff008800, //
                0xff77ff77, 0xff0000ff, 0xffff0000, 0xff008800, //
            ];

            for y in 0..64 {
                // The last three columns in the video buffer are special.
                // 127 - unknown
                // 126 - bit 1 = palette bit 1
                // 125 - bit 1 = palette bit 0 (or with 126 bit 0)
                // (palette is shifted by two and added to 'color'
                //  to find palette index which holds the color's index)
                let palette = ((pixels[y * 128 + 125] & 2 >> 1) | (pixels[y * 128 + 126])) & 0b11;

                for x in 0..128 {
                    let pixel = pixels[y * 128 + x];

                    let color = COLORS[(pixel + palette * 4) as usize];

                    let addr = (y * 128 * 2 * 2) + (x * 2);
                    buffer[addr + 0] = color;
                    buffer[addr + 1] = color;
                    buffer[addr + 256] = color;
                    buffer[addr + 257] = color;
                }
            }

            // We unwrap here as we want this code to exit if it fails
            window.update_with_buffer(&buffer, WIDTH, HEIGHT).unwrap();
        }
    }
}

struct DesktopChannelF {
    sound: RefCell<Sound>,

    pixels: RefCell<[u8; 128 * 64]>,

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

use chf_emulator::ChannelF;
use chf_emulator::Sound;
use rodio::source::SineWave;

impl<'a> ChannelF for DesktopChannelF {
    fn sound(&self, frequency: Sound) {
        *self.sound.borrow_mut() = frequency;
    }

    fn set_pixel(&self, x: u8, y: u8, value: u8) {
        let x = x;
        let y = y;
        self.pixels.borrow_mut()[x as usize + y as usize * 128usize] = value;
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
