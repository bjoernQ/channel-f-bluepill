#![no_std]

use core::{usize};

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Sound {
    Silence,
    Frequency1Khz,
    Frequency500Hz,
    Frequency120Hz,
}

pub enum Key {
    Reset,

    Time,
    Mode,
    Hold,
    Start,

    Right0,
    Left0,
    Back0,
    Forward0,
    CounterClockwise0,
    Clockwise0,
    Pull0,
    Push0,

    Right1,
    Left1,
    Back1,
    Forward1,
    CounterClockwise1,
    Clockwise1,
    Pull1,
    Push1,
}

pub trait ChannelF {
    fn sound(&self, frequency: Sound);

    fn set_pixel(&self, x: u8, y: u8, value: u8);

    fn key_pressed(&self, key: Key) -> bool;
}

pub struct Cpu<'a> {
    pub a: u8, // accumulator
    pub scratchpad: [u8; 64],
    pub isar: u8, // scratchpad address (ISAR)
    pub flags: u8, // (0x1: SF, 0x2: CF, 0x4: ZF, 0x8: OF)
    pub icb_flag: u8, // 0x10: ICB

    pub pc0: u16,
    pub pc1: u16,
    pub dc0: u16,
    pub dc1: u16,

    pub cycles: u16,

    memory_0000: &'a [u8],
    memory_0400: &'a [u8],
    memory_0800: &'a [u8],
    ram_2800: [u8; 0x800], // 0x2800 ..= 0x3000

    pub xmemory: [u8; 0x400],
    xregister: u16,

    channel_f: &'a dyn ChannelF,

    io_latch: [u8; 256],

    x: u8,
    y: u8,
    color: u8,
}

impl<'a> Cpu<'a> {
    pub fn new(
        rom_0: &'a [u8],
        rom_400: &'a [u8],
        cartridge: &'a [u8],
        channel_f: &'a dyn ChannelF,
    ) -> Cpu<'a> {
        Cpu {
            a: 0,
            scratchpad: [0u8; 64],
            isar: 0,
            flags: 0,
            icb_flag: 0,
            pc0: 0,
            pc1: 0,
            dc0: 0,
            dc1: 0,
            cycles: 0,
            memory_0000: rom_0,
            memory_0400: rom_400,
            memory_0800: cartridge,
            ram_2800: [0u8; 0x800],

            xmemory: [1u8; 0x400],
            xregister: 0u16,

            channel_f: channel_f,
            io_latch: [0u8; 256],

            x: 0,
            y: 0,
            color: 0,
        }
    }

    pub fn reset(&mut self) {
        self.pc0 = 0;
        self.pc1 = 0;
    }

    fn result_0czs0o(&mut self, v: u8) -> u8 {
        self.flags = 0;
        if v < 0x80 {
            self.flags += 0x1 // SF
        }

        if v == 0 {
            self.flags += 0x4 // ZF
        }

        v
    }

    fn add_czso(&mut self, v1: u8, v2: u8, c: u8) -> u8 {
        let full_res = v1 as u16 + v2 as u16 + c as u16;
        let res = (full_res & 0xff) as u8;

        self.flags = 0;

        if res < 0x80 {
            self.flags |= 0x1; // SF
        }

        if res == 0 {
            self.flags |= 0x4 // ZF
        }

        if (full_res & 0x100) != 0 {
            self.flags |= 0x2 // CF
        }

        if ((v1 ^ res) & (v2 ^ res) & 0x80) != 0 {
            self.flags |= 0x8; // OF
        }

        res
    }

    fn adddec(&mut self, v1: u8, v2: u8) -> u8 {
        /* From F8 Guide To programming description of AMD
         * binary add the addend to the binary sum of the augend and $66
         * *NOTE* the binary addition of the augend to $66 is done before AMD is called
         * record the status of the carry and intermediate carry
         * add a factor to the sum based on the carry and intermediate carry:
         * - no carry, no intermediate carry, add $AA
         * - no carry, intermediate carry, add $A0
         * - carry, no intermediate carry, add $0A
         * - carry, intermediate carry, add $00
         * any carry from the low-order digit is suppressed
         * *NOTE* status flags are updated prior to the factor being added
         */
        let augend = v1;
        let addend = v2;

        let mut tmp = augend.overflowing_add(addend).0;

        let mut c = 0u8; // high order carry
        let mut ic = 0u8; // low order carry

        if ((augend as u16 + addend as u16) & 0xff0) > 0xf0 {
            c = 1;
        }

        if (augend & 0x0f) + (addend & 0x0f) > 0x0F {
            ic = 1;
        }

        self.flags = 0;
        self.add_czso(augend, addend, 0);

        if c == 0 && ic == 0 {
            tmp = ((tmp.overflowing_add(0xa0).0) & 0xf0) + ((tmp.overflowing_add(0x0a).0) & 0x0f);
        }

        if c == 0 && ic == 1 {
            tmp = ((tmp.overflowing_add(0xa0).0) & 0xf0) + (tmp & 0x0f);
        }

        if c == 1 && ic == 0 {
            tmp = (tmp & 0xf0) + ((tmp.overflowing_add(0x0a).0) & 0x0f);
        }

        return tmp;
    }

    fn cmp(&mut self, v1: u8, v2: u8) {
        self.add_czso(v2, v1 ^ 0xff, 1);
    }

    fn branch(&mut self, cond: bool) {
        let offset = signed_byte(self.fetch()) - 1;
        if cond {
            if offset > 0 {
                self.pc0 = self.pc0.overflowing_add(offset as u16).0;
                //self.pc0 += offset as u16;
            } else {
                self.pc0 = self.pc0.overflowing_sub((offset * -1) as u16).0;
                //self.pc0 -= (offset * -1) as u16;
            }
            self.cycles += 2;
        }
    }

    fn inc_isl(&mut self) {
        self.isar = ((self.isar.overflowing_add(1).0) & 0x7) + (self.isar & 0x38);
    }

    fn dec_isl(&mut self) {
        self.isar = ((self.isar.overflowing_sub(1).0) & 0x7) + (self.isar & 0x38);
    }

    pub fn fetch(&mut self) -> u8 {
        let res = self.get_from_memory(self.pc0);
        self.pc0 += 1;
        res
    }

    fn get_from_memory(&self, addr: u16) -> u8 {
        let addr = addr as usize;
        match addr {
            0..=0x3ff => self.memory_0000[addr],
            0x400..=0x7ff => self.memory_0400[addr - 0x400],
            0x800..=0x27ff => {
                if self.memory_0800.len() > addr - 0x800 {
                    self.memory_0800[addr - 0x800]
                } else {
                    0xff
                }
            }
            0x2800..=0x2fff => self.ram_2800[addr - 0x2800],
            _ => 0xff,
        }
    }

    fn read(&self, addr: u16) -> u8 {
        if addr < 0x4000 {
            self.get_from_memory(addr)
        } else {
            0xff
        }
    }

    fn write(&mut self, _addr: u16, _v: u8) {
        todo!("write");
    }

    fn inport(&mut self, port: u8) -> u8 {
        if port == 0 {
            let mut res = 0xf;

            if self.channel_f.key_pressed(Key::Start) {
                res ^= 0x1;
            }

            if self.channel_f.key_pressed(Key::Hold) {
                res ^= 0x2;
            }

            if self.channel_f.key_pressed(Key::Mode) {
                res ^= 0x4;
            }

            if self.channel_f.key_pressed(Key::Time) {
                res ^= 0x8;
            }

            return res;
        }

        if port == 1 {
            let mut input = 0;

            if self.channel_f.key_pressed(Key::Right0) {
                input += 1;
            }

            if self.channel_f.key_pressed(Key::Left0) {
                input += 2;
            }

            if self.channel_f.key_pressed(Key::Back0) {
                input += 4;
            }

            if self.channel_f.key_pressed(Key::Forward0) {
                input += 8;
            }

            if self.channel_f.key_pressed(Key::CounterClockwise0) {
                input += 16;
            }

            if self.channel_f.key_pressed(Key::Clockwise0) {
                input += 32;
            }

            if self.channel_f.key_pressed(Key::Pull0) {
                input += 64;
            }

            if self.channel_f.key_pressed(Key::Push0) {
                input += 128;
            }

            if self.io_latch[0] & 0x40 == 0 {
                return input ^ 0xff;
            }
        }

        if port == 4 {
            let mut input = 0;

            if self.channel_f.key_pressed(Key::Right1) {
                input += 1;
            }

            if self.channel_f.key_pressed(Key::Left1) {
                input += 2;
            }

            if self.channel_f.key_pressed(Key::Back1) {
                input += 4;
            }

            if self.channel_f.key_pressed(Key::Forward1) {
                input += 8;
            }

            if self.channel_f.key_pressed(Key::CounterClockwise1) {
                input += 16;
            }

            if self.channel_f.key_pressed(Key::Clockwise1) {
                input += 32;
            }

            if self.channel_f.key_pressed(Key::Pull1) {
                input += 64;
            }

            if self.channel_f.key_pressed(Key::Push1) {
                input += 128;
            }

            if self.io_latch[0] & 0x40 == 0 {
                return input ^ 0xff;
            }
        }

        if port == 0x20 || port == 0x24 {
            self.xmemory_update();
            return  ((self.xregister.overflowing_shr(8).0) & 0xff) as u8 | self.io_latch[port as usize];
        }

        if port == 0x21 || port == 0x25 {
            self.xmemory_update();
            return  (self.xregister & 0xff) as u8 | self.io_latch[port as usize];
        }

        self.io_latch[port as usize]
    }

    fn outport(&mut self, port: u8, v: u8) {
        let old = self.io_latch[port as usize];
        self.io_latch[port as usize] = v;

        match port {
            1 => self.color = v.overflowing_shr(6).0 ^ 0x3,
            4 => self.x = (v & 0x7f) ^ 0x7f,
            5 => {
                self.y = (v & 0x3f) ^ 0x3f;

                self.channel_f.sound(match v.overflowing_shr(6).0 {
                    0 => Sound::Silence,
                    1 => Sound::Frequency1Khz,
                    2 => Sound::Frequency500Hz,
                    3 => Sound::Frequency120Hz,
                    _ => Sound::Silence,
                });
            }
            0 => {
                if v & 0x20 == 0 && old & 0x20 != 0 {
                    self.channel_f.set_pixel(self.x, self.y, self.color);
                }
            }
            0x20 | 0x24 => {
                self.xregister = (self.xregister & 0xff) + ((v as u16 & 0xf).overflowing_shl(8).0);
                self.xmemory_update();
            }
            0x21 | 0x25 => {
                self.xregister = (self.xregister & 0xff00) + (v as u16);
                self.xmemory_update();
            }
            _ => (),
        }
    }

    fn xmemory_update(&mut self) {
        let addr = (self.xregister & 0xff) + ((self.xregister.overflowing_shr(1).0) & 0x300);
        if self.xregister & 0x100 != 0 {
            self.xmemory[addr as usize] = ((self.xregister.overflowing_shr(11).0) & 1) as u8;
        } else {
            self.xregister = (self.xregister & 0xfff) + (self.xmemory[addr as usize].overflowing_shl(15).0 as u16);
        }
    }

    pub fn execute(&mut self, opcode: u8) {
        match opcode {
            // LR
            0x00..=0x03 => {
                self.cycles += 4;
                self.a = self.scratchpad[12usize + opcode as usize];
            }
            0x04..=0x07 => {
                self.cycles += 4;
                self.scratchpad[12usize + (opcode - 0x04) as usize] = self.a;
            }
            0x08 => {
                self.cycles += 16;
                self.scratchpad[12] = (self.pc1.overflowing_shr(8).0 & 0xff) as u8;
                self.scratchpad[13] = (self.pc1 & 0xff) as u8;
            }
            0x09 => {
                self.cycles += 16;
                self.pc1 =
                    (self.scratchpad[12] as u16).overflowing_shl(8).0 + self.scratchpad[13] as u16;
            }
            0x0a => {
                self.cycles += 4;
                self.a = self.isar;
            }
            0x0b => {
                self.cycles += 4;
                self.isar = self.a & 0x3f;
            }
            // PK
            0x0c => {
                self.cycles += 16;
                self.pc1 = self.pc0;
                self.pc0 = (self.scratchpad[12] as u16)
                    .overflowing_shl(8)
                    .0
                    .overflowing_add(self.scratchpad[13] as u16)
                    .0;
            }
            // LR
            0x0d => {
                self.cycles += 16;
                self.pc0 = (self.scratchpad[14] as u16)
                    .overflowing_shl(8)
                    .0
                    .overflowing_add(self.scratchpad[15] as u16)
                    .0;
            }
            0x0e => {
                self.cycles += 16;
                self.scratchpad[14] = (self.dc0.overflowing_shr(8).0) as u8;
                self.scratchpad[15] = (self.dc0 & 0xff) as u8;
            }
            0x0f => {
                self.cycles += 16;
                self.dc0 = (self.scratchpad[14] as u16)
                    .overflowing_shl(8)
                    .0
                    .overflowing_add(self.scratchpad[15] as u16)
                    .0;
            }
            0x10 => {
                self.cycles += 16;
                self.dc0 = (self.scratchpad[10] as u16)
                    .overflowing_shl(8)
                    .0
                    .overflowing_add(self.scratchpad[11] as u16)
                    .0;
            }
            0x11 => {
                self.cycles += 16;
                self.scratchpad[10] = self.dc0.overflowing_shr(8).0 as u8;
                self.scratchpad[11] = (self.dc0 & 0xff) as u8;
            }
            // SR 1
            0x12 => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a.overflowing_shr(1).0 & 0xff);

                self.flags = (self.flags | 0b0001) & 0b0101;
            }
            // SL 1
            0x13 => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a.overflowing_shl(1).0 & 0xff);
                self.flags = self.flags & 0b0101;
            }
            // SR 4
            0x14 => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a.overflowing_shr(4).0 & 0xff);

                self.flags = (self.flags | 0b0001) & 0b0101;
            }
            // SL 4
            0x15 => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a.overflowing_shl(4).0 & 0xff);
                self.flags = self.flags & 0b0101;
            }
            // LM
            0x16 => {
                self.cycles += 10;
                self.a = self.read(self.dc0);
                self.dc0 = self.dc0.overflowing_add(1).0;
            }
            // ST
            0x17 => {
                self.cycles += 10;
                self.write(self.dc0, self.a);
                self.dc0 = self.dc0.overflowing_add(1).0;
            }
            // COM
            0x18 => {
                self.cycles += 4;
                self.a = self.result_0czs0o(!self.a);
                self.flags = self.flags & 0b0101;
            }
            // LNK
            0x19 => {
                self.cycles += 4;
                self.a = self.add_czso(self.a, 0, self.flags.overflowing_shr(1).0 & 1);
            }
            // DI
            0x1a => {
                self.cycles += 8;
                self.icb_flag = 0;
            }
            // EI
            0x1b => {
                self.cycles += 8;
                self.icb_flag = 0x10;
            }
            // POP
            0x1c => {
                self.cycles += 8;
                self.pc0 = self.pc1;
            }
            // LR
            0x1d => {
                self.cycles += 8;
                self.flags = self.scratchpad[9] & 0xf;
                self.icb_flag = self.scratchpad[9] & 0x10;
            }
            0x1e => {
                self.cycles += 4;
                self.scratchpad[9] = self.flags + self.icb_flag;
            }
            // INC
            0x1f => {
                self.cycles += 4;
                self.a = self.add_czso(self.a, 0, 1);
            }
            // LI
            0x20 => {
                self.cycles += 10;
                self.a = self.fetch();
            }
            // NI
            0x21 => {
                self.cycles += 10;
                let fetched = self.fetch();
                self.a = self.result_0czs0o(self.a & fetched);
            }
            // OI
            0x22 => {
                self.cycles += 10;
                let fetched = self.fetch();
                self.a = self.result_0czs0o(self.a | fetched);
            }
            // XI
            0x23 => {
                self.cycles += 10;
                let fetched = self.fetch();
                self.a = self.result_0czs0o(self.a ^ fetched);
            }
            // AI
            0x24 => {
                self.cycles += 10;
                let fetched = self.fetch();
                self.a = self.add_czso(self.a, fetched, 0);
            }
            // CI
            0x25 => {
                self.cycles += 10;
                let fetched = self.fetch();
                self.cmp(self.a, fetched);
            }
            // IN
            0x26 => {
                self.cycles += 16;
                let port = self.fetch();
                let in_res = self.inport(port);
                self.a = self.result_0czs0o(in_res);
            }
            // OUT
            0x27 => {
                self.cycles += 16;
                let fetched = self.fetch();
                self.outport(fetched, self.a);
            }
            // PI
            0x28 => {
                self.cycles += 0x1a;
                self.a = self.fetch();
                let tmp = self.fetch();
                self.pc1 = self.pc0;
                self.pc0 = (self.a as u16).overflowing_shl(8).0 + tmp as u16;
            }
            // JMP
            0x29 => {
                self.cycles += 16;
                self.a = self.fetch();
                let tmp = self.fetch() as u16;
                self.pc0 = (self.a as u16).overflowing_shl(8).0 + tmp;
            }
            // DCI
            0x2a => {
                self.cycles += 0x18;
                self.dc0 = (self.fetch() as u16).overflowing_shl(8).0;
                self.dc0 += self.fetch() as u16;
            }
            // NOP
            0x2b => {
                self.cycles += 4;
            }
            // XDC
            0x2c => {
                self.cycles += 8;
                let x = self.dc0;
                self.dc0 = self.dc1;
                self.dc1 = x;
            }
            // DS
            0x30..=0x3b => {
                self.cycles += 6;
                let index = opcode as usize - 0x30usize;
                self.scratchpad[index] = self.add_czso(self.scratchpad[index], 0xff, 0);
            }
            0x3c => {
                self.cycles += 6;
                self.scratchpad[self.isar as usize] =
                    self.add_czso(self.scratchpad[self.isar as usize], 0xff, 0);
            }
            0x3d => {
                self.cycles += 6;
                self.scratchpad[self.isar as usize] =
                    self.add_czso(self.scratchpad[self.isar as usize], 0xff, 0);
                self.inc_isl();
            }
            0x3e => {
                self.cycles += 6;
                self.scratchpad[self.isar as usize] =
                    self.add_czso(self.scratchpad[self.isar as usize], 0xff, 0);
                self.dec_isl();
            }
            // LR
            0x40..=0x4b => {
                self.cycles += 4;
                let index = opcode as usize - 0x40;
                self.a = self.scratchpad[index];
            }
            0x4c => {
                self.cycles += 4;
                self.a = self.scratchpad[self.isar as usize];
            }
            0x4d => {
                self.cycles += 4;
                self.a = self.scratchpad[self.isar as usize];
                self.inc_isl();
            }
            0x4e => {
                self.cycles += 4;
                self.a = self.scratchpad[self.isar as usize];
                self.dec_isl();
            }
            0x50..=0x5b => {
                self.cycles += 4;
                let index = opcode as usize - 0x50usize;
                self.scratchpad[index] = self.a;
            }
            0x5c => {
                self.cycles += 4;
                self.scratchpad[self.isar as usize] = self.a;
            }
            0x5d => {
                self.cycles += 4;
                self.scratchpad[self.isar as usize] = self.a;
                self.inc_isl();
            }
            0x5e => {
                self.cycles += 4;
                self.scratchpad[self.isar as usize] = self.a;
                self.dec_isl();
            }
            // LISU
            0x60..=0x67 => {
                self.cycles += 4;
                let index = opcode - 0x60;
                self.isar = (self.isar & 0x7) + (index & 0x7).overflowing_shl(3).0;
            }
            // LISL
            0x68..=0x6f => {
                self.cycles += 4;
                let index = opcode - 0x68;
                self.isar = (self.isar & 0x38) + index;
            }
            // LIS
            0x70..=0x7f => {
                self.cycles += 4;
                self.a = opcode - 0x70;
            }
            // BT
            0x80..=0x87 => {
                self.cycles += 0xc;
                let index = opcode - 0x80;
                self.branch((self.flags & index) != 0);
            }
            // AM
            0x88 => {
                self.cycles += 10;
                self.a = self.add_czso(self.a, self.read(self.dc0), 0);
                self.dc0 = self.dc0.overflowing_add(1).0;
            }
            // AMD
            0x89 => {
                self.cycles += 10;
                self.a = self.adddec(self.a, self.read(self.dc0));
                self.dc0 = self.dc0.overflowing_add(1).0;
            }
            // NM
            0x8a => {
                self.cycles += 10;
                self.a = self.result_0czs0o(self.a & self.read(self.dc0));
                self.dc0 = self.dc0.overflowing_add(1).0;
                self.flags = self.flags & 0b0101;
            }
            // OM
            0x8b => {
                self.cycles += 10;
                self.a = self.result_0czs0o(self.a | self.read(self.dc0));
                self.dc0 = self.dc0.overflowing_add(1).0;
                self.flags = self.flags & 0b0101;
            }
            // XM
            0x8c => {
                self.cycles += 10;
                self.a = self.result_0czs0o(self.a ^ self.read(self.dc0));
                self.dc0 = self.dc0.overflowing_add(1).0;
                self.flags = self.flags & 0b0101;
            }
            // CM
            0x8d => {
                self.cycles += 10;
                self.cmp(self.a, self.read(self.dc0));
                self.dc0 = self.dc0.overflowing_add(1).0;
            }
            // ADC
            0x8e => {
                self.cycles += 10;
                let signed_a = signed_byte(self.a);
                if signed_a > 0 {
                    self.dc0 += signed_a as u16;
                } else {
                    self.dc0 -= (signed_a * -1) as u16;
                }
            }
            // BR7
            0x8f => {
                self.cycles += 8;
                self.branch(self.isar & 0x7 != 0x7);
            }
            // BF
            0x90..=0x9f => {
                self.cycles += 0xc;
                let index = opcode - 0x90;
                self.branch(self.flags & index == 0);
            }
            // INS
            0xa0..=0xa1 => {
                self.cycles += 8;
                let index = opcode - 0xa0;
                let input = self.inport(index);
                self.a = self.result_0czs0o(input);
                self.flags = self.flags & 0b0101;
            }
            0xa2..=0xaf => {
                self.cycles += 0x10;
                let index = opcode - 0xa0;
                let input = self.inport(index);
                self.a = self.result_0czs0o(input);
                self.flags = self.flags & 0b0101;
            }
            // OUTS
            0xb0..=0xb1 => {
                self.cycles += 8;
                let index = opcode - 0xb0;
                self.outport(index, self.a);
            }
            0xb2..=0xbf => {
                self.cycles += 0x10;
                let index = opcode - 0xb0;
                self.outport(index, self.a);
            }
            // AS
            0xc0..=0xcb => {
                self.cycles += 4;
                let index = (opcode - 0xc0) as usize;
                self.a = self.add_czso(self.a, self.scratchpad[index], 0);
            }
            0xcc => {
                self.cycles += 4;
                self.a = self.add_czso(self.a, self.scratchpad[self.isar as usize], 0);
            }
            0xcd => {
                self.cycles += 4;
                self.a = self.add_czso(self.a, self.scratchpad[self.isar as usize], 0);
                self.inc_isl();
            }
            0xce => {
                self.cycles += 4;
                self.a = self.add_czso(self.a, self.scratchpad[self.isar as usize], 0);
                self.dec_isl();
            }
            // ASD
            0xd0..=0xdb => {
                self.cycles += 8;
                let index = (opcode - 0xd0) as usize;
                self.a = self.adddec(self.a, self.scratchpad[index]);
            }
            0xdc => {
                self.cycles += 8;
                self.a = self.adddec(self.a, self.scratchpad[self.isar as usize]);
            }
            0xdd => {
                self.cycles += 8;
                self.a = self.adddec(self.a, self.scratchpad[self.isar as usize]);
                self.inc_isl();
            }
            0xde => {
                self.cycles += 8;
                self.a = self.adddec(self.a, self.scratchpad[self.isar as usize]);
                self.dec_isl();
            }
            // XS
            0xe0..=0xeb => {
                self.cycles += 4;
                let index = (opcode - 0xe0) as usize;
                self.a = self.result_0czs0o(self.a ^ self.scratchpad[index]);
                self.flags = self.flags & 0b0101;
            }
            0xec => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a ^ self.scratchpad[self.isar as usize]);
                self.flags = self.flags & 0b0101;
            }
            0xed => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a ^ self.scratchpad[self.isar as usize]);
                self.inc_isl();
                self.flags = self.flags & 0b0101;
            }
            0xee => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a ^ self.scratchpad[self.isar as usize]);
                self.dec_isl();
                self.flags = self.flags & 0b0101;
            }
            // NS
            0xf0..=0xfb => {
                self.cycles += 4;
                let index = (opcode - 0xf0) as usize;
                self.a = self.result_0czs0o(self.a & self.scratchpad[index]);
                self.flags = self.flags & 0b0101;
            }
            0xfc => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a & self.scratchpad[self.isar as usize]);
                self.flags = self.flags & 0b0101;
            }
            0xfd => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a & self.scratchpad[self.isar as usize]);
                self.inc_isl();
                self.flags = self.flags & 0b0101;
            }
            0xfe => {
                self.cycles += 4;
                self.a = self.result_0czs0o(self.a & self.scratchpad[self.isar as usize]);
                self.dec_isl();
                self.flags = self.flags & 0b0101;
            }
            _ =>  panic!("Unknown opcode {:x}", opcode),
        }
    }
}

fn signed_byte(v: u8) -> i8 {
    if v < 0x80 {
        v as i8
    } else {
        ((!v as i8) * -1) - 1
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use core::cell::RefCell;
    use std::prelude::v1::*;

    use super::*;

    const ROM_0000: &'static [u8] = include_bytes!("../roms/SL31253.bin");
    const ROM_0400: &'static [u8] = include_bytes!("../roms/SL31254.bin");
    const CARTRIDGE: &'static [u8] = include_bytes!("../roms/demo.bin");
    const CARTRIDGE_TEST: &'static [u8] = include_bytes!("../testfiles/test.bin");

    struct DummyChannelF {
        pixels: RefCell<[u8; 128 * 64]>,

        key_pressed: RefCell<bool>,
    }

    impl ChannelF for DummyChannelF {
        fn sound(&self, _frequency: Sound) {}

        fn set_pixel(&self, x: u8, y: u8, value: u8) {
            self.pixels.borrow_mut()[x as usize + y as usize * 128usize] = value;
        }

        fn key_pressed(&self, key: Key) -> bool {
            match key {
                Key::Start => self.key_pressed.borrow().clone(),
                _ => false,
            }
        }
    }

    #[test]
    fn create_cpu() {
        let dummy_channel_f = DummyChannelF {
            pixels: RefCell::new([0u8; 128 * 64]),
            key_pressed: RefCell::new(false),
        };
        let mut cpu = Cpu::new(&[], &[], &[], &dummy_channel_f);
        cpu.reset();
    }

    #[test]
    fn signed_to_unsigned() {
        assert_eq!(0x20, signed_byte(0x20));
        assert_eq!(0x7f, signed_byte(0x7f));
        assert_eq!(-2, signed_byte(0b11111110));
        assert_eq!(-126, signed_byte(0b10000010));
        assert_eq!(-8, signed_byte(0xf8));
    }

    #[test]
    fn add() {
        let dummy_channel_f = DummyChannelF {
            pixels: RefCell::new([0u8; 128 * 64]),
            key_pressed: RefCell::new(false),
        };
        let mut cpu = Cpu::new(&[], &[], &[], &dummy_channel_f);
        cpu.reset();
        
        assert_eq!(cpu.add_czso(0x07, 0xff,0), 0x06);
        assert_eq!(cpu.add_czso(0x07, 0x01,0), 0x08);

        assert_eq!(cpu.add_czso(0xff, 0,1), 0x00);
        assert!( cpu.flags & 0x4 != 0 );

        assert_eq!(cpu.add_czso(0xfe, 0,1), 0xff);
        assert!( cpu.flags & 0x4 == 0 );

    }

    #[test]
    fn run_some_opcodes() {
        let dummy_channel_f = DummyChannelF {
            pixels: RefCell::new([0u8; 128 * 64]),
            key_pressed: RefCell::new(false),
        };
        let mut cpu = Cpu::new(&[], &[], &[], &dummy_channel_f);
        cpu.reset();

        cpu.a = 0x8f;
        cpu.execute(0x50);
        assert_eq!(0x8f, cpu.scratchpad[0]);
        assert_eq!(4, cpu.cycles);
    }

    #[test]
    fn startup() {
        let dummy_channel_f = DummyChannelF {
            pixels: RefCell::new([0u8; 128 * 64]),
            key_pressed: RefCell::new(false),
        };
        let catridge = CARTRIDGE;
        let mut cpu = Cpu::new(ROM_0000, ROM_0400, catridge, &dummy_channel_f);
        cpu.reset();

        for _ in 0..55591320 {
            cpu.cycles = 0;
            let opcode = cpu.fetch();
            cpu.execute(opcode);
        }

        let mut screen = String::new();
        for y in 0..64 {
            screen.push('[');
            for x in 0..128 {
                let p = dummy_channel_f.pixels.borrow_mut()[(y * 128 + x) as usize] & 0xf;

                let x = match p {
                    3 => '█',
                    2 => 'X',
                    1 => '|',
                    0 => ' ',
                    _ => '?',
                };
                screen.push(x);
            }
            screen.push(']');
        }

        assert_eq!(
"[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[          █████ █████ █████ █████ █████ █   █ █████ █     █████                                                               █ ]\
[          █     █   █   █   █   █ █     █   █   █   █      █  █                                                               █ ]\
[          ███   █████   █   █████ █     █████   █   █      █  █                                                               █ ]\
[          █     █   █   █   █  █  █     █   █   █   █      █  █                                                               █ ]\
[          █     █   █ █████ █   █ █████ █   █ █████ █████ █████                                                               █ ]\
[                                                                                                                              █ ]\
[          █   █ █████ █████ █████ █████                                                                                       █ ]\
[          █   █   █    █  █ █     █   █                                                                                       █ ]\
[          █   █   █    █  █ ███   █   █                                                                                       █ ]\
[           █ █    █    █  █ █     █   █                                                                                       █ ]\
[            █   █████ █████ █████ █████                                                                                       █ ]\
[                                                                                                                              █ ]\
[          █████ █   █ █████ █████ █████ █████ █████ █████ █   █ █   █ █████ █   █ █████                                       █ ]\
[          █     ██  █   █   █     █   █   █   █   █   █   ██  █ ██ ██ █     ██  █   █                                         █ ]\
[          ███   █ █ █   █   ███   █████   █   █████   █   █ █ █ █ █ █ ███   █ █ █   █                                         █ ]\
[          █     █  ██   █   █     █  █    █   █   █   █   █  ██ █   █ █     █  ██   █                                         █ ]\
[          █████ █   █   █   █████ █   █   █   █   █ █████ █   █ █   █ █████ █   █   █                                         █ ]\
[                                                                                                                              █ ]\
[          █████ █████ █   █ █████ █████ █████       █████ █     █████ █████ █████ █████                                       █ ]\
[          █     █     ██  █   █   █     █   █       █   █ █     █     █   █ █     █                                           █ ]\
[          █     ███   █ █ █   █   ███   █████       █████ █     ███   █████ █████ ███                                         █ ]\
[          █     █     █  ██   █   █     █  █        █     █     █     █   █     █ █                                           █ ]\
[          █████ █████ █   █   █   █████ █   █       █     █████ █████ █   █ █████ █████                                       █ ]\
[                                                                                                                              █ ]\
[          █████ █   █ █████ █   █       █████ █   █ █████ █████ █████ █   █         X                                         █ ]\
[          █   █ █   █ █     █   █        █  █ █   █   █     █   █   █ ██  █        XX                                         █ ]\
[          █████ █   █ █████ █████        ████ █   █   █     █   █   █ █ █ █         X                                         █ ]\
[          █     █   █     █ █   █        █  █ █   █   █     █   █   █ █  ██         X                                         █ ]\
[          █     █████ █████ █   █       █████ █████   █     █   █████ █   █        XXX                                        █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]\
[                                                                                                                              █ ]",
            screen
        );
    }

    #[test]
    fn startup_no_cartridge() {
        let dummy_channel_f = DummyChannelF {
            pixels: RefCell::new([0u8; 128 * 64]),
            key_pressed: RefCell::new(false),
        };
        let mut cpu = Cpu::new(ROM_0000, ROM_0400, &[], &dummy_channel_f);
        cpu.reset();

        for _ in 0..55591320 {
            cpu.cycles = 0;
            let opcode = cpu.fetch();
            cpu.execute(opcode);
        }

        assert_eq!(195, cpu.pc0);
    }

    #[test]
    fn test_generated() {
        let mut pcs: Vec<String> = Vec::new();
        let f = std::fs::File::open("./testfiles/test.log").unwrap();
        let file = std::io::BufReader::new(&f);
        for line in std::io::BufRead::lines(file) {
            let line = line.unwrap();
            pcs.push( line );
        } 


        let dummy_channel_f = DummyChannelF {
            pixels: RefCell::new([0u8; 128 * 64]),
            key_pressed: RefCell::new(false),
        };
        let catridge = CARTRIDGE_TEST;
        let mut cpu = Cpu::new(ROM_0000, ROM_0400, catridge, &dummy_channel_f);
        cpu.reset();


        let mut pcs_idx = 0usize;
        let mut checking = false;
        for _ in 0..55591320 {

            if cpu.pc0 == 0x803 && !checking {
                checking = true;
            }

            if cpu.pc0 == 0x1000 {
                break;
            }

            if checking {
                let current = std::format!(
                    "A={:02X} W={:02X} IS={:02X} R0={:02X} R1={:02X} R2={:02X} R3={:02X} R4={:02X} {:04X}: ", 
                    cpu.a, 
                    cpu.flags | cpu.icb_flag,
                    cpu.isar,
                    cpu.scratchpad[0],
                    cpu.scratchpad[1],
                    cpu.scratchpad[2],
                    cpu.scratchpad[3],
                    cpu.scratchpad[4],
                    cpu.pc0,
                );


                if !pcs[pcs_idx].starts_with(&current) {
                    std::println!("should be \n{} but is \n{}, index={}", &pcs[pcs_idx], &current, pcs_idx);
                    assert!(false);
                }
                pcs_idx += 1;
            }

            cpu.cycles = 0;
            let opcode = cpu.fetch();
            cpu.execute(opcode);
        }

    }

    // memory
    // 0x0000 BIOS (BIOS SL31253 or SL90025 to location 0x0, BIOS SL31254 to location 0x400)
    // 0x0800 Cartridge ID (0x55 0x08)
    // 0x0802 Cartrige Start Address
    // 0x2800 additional RAM on cartridge
}
