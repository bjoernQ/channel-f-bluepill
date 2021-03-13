use cortex_m::Peripherals;
use stm32f1xx_hal::pac::{interrupt, Interrupt};
use stm32f1xx_hal::{
    gpio::{gpioa::PA0, Output, PushPull},
    pac,
};

use core::mem::MaybeUninit;

static mut TIMER_TIM4: MaybeUninit<pac::TIM4> = MaybeUninit::uninit();
static mut TIMER_TIM1: MaybeUninit<pac::TIM1> = MaybeUninit::uninit();
static mut BUSY_PIN: MaybeUninit<PA0<Output<PushPull>>> = MaybeUninit::uninit();

pub static mut VID_RAM: [u8; 128 * 64] = [0u8; 128 * 64];

const ISR_OVERHEAD_CORRECTION: u16 = 110;

pub fn init_video(
    cp: &mut Peripherals,
    tim4: pac::TIM4,
    tim1: pac::TIM1,
    busy: PA0<Output<PushPull>>,
) {
    unsafe {
        pac::NVIC::unmask(Interrupt::TIM4);
        pac::NVIC::unmask(Interrupt::TIM1_UP);

        cp.NVIC.set_priority(Interrupt::TIM4, 16);
        cp.NVIC.set_priority(Interrupt::TIM1_UP, 32);
    }

    unsafe {
        (*pac::RCC::ptr())
            .apb1enr
            .modify(|_, w| w.tim4en().set_bit());

        (*pac::RCC::ptr())
            .apb2enr
            .modify(|_, w| w.tim1en().set_bit());
    }

    // configure TIM4
    configure_tim4(&tim4);

    // configure TIM1
    configure_tim1(&tim1);

    // make peripherals accessible from the isr
    unsafe {
        let timer_static = TIMER_TIM4.as_mut_ptr();
        *timer_static = tim4;

        let timer_static = TIMER_TIM1.as_mut_ptr();
        *timer_static = tim1;

        *(BUSY_PIN.as_mut_ptr()) = busy;
    }
}

pub fn start_video() {
    schedule(HALF_SCANLINE_ARR, SHORT_SYNC_CRR);
}

#[inline(always)]
fn schedule(new_arr: u16, new_crr: u16) {
    unsafe {
        let tim1 = TIMER_TIM1.as_mut_ptr();
        (*tim1).cnt.write(|w| w.bits(0));
        (*tim1).arr.modify(|_, w| w.arr().bits(new_arr - 15));
        // start timer
        (*tim1).cr1.modify(|_, w| {
            w.cen().set_bit() // START!
        });

        let tim = TIMER_TIM4.as_mut_ptr();
        (*tim).arr.modify(|_, w| w.arr().bits(new_arr));
        (*tim).ccr4.write(|w| w.bits(new_crr as u32));
        // start anew
        (*tim).cr1.modify(|_, w| w.cen().set_bit());
    }
}

fn configure_tim4(tim: &stm32f1xx_hal::pac::TIM4) {
    tim.arr.modify(|_, w| {
        w.arr().bits(0) // right value in schedule
    });

    tim.ccr4.modify(|_, w| {
        w.ccr().bits(0) // right value in schedule
    });

    tim.psc.modify(|_, w| {
        w.psc().bits(0) // no prescaler
    });

    // pwm mode etc
    tim.ccmr2_output_mut().modify(|_, w| {
        w.oc4m()
            .pwm_mode2() // pwm mode low/high
            .oc4pe()
            .clear_bit() // disable output compare preload
            .oc4fe()
            .set_bit() // enable fast mode
            .cc4s()
            .output()
    });

    // output enable channel 4
    tim.ccer.modify(|_, w| w.cc4e().set_bit());

    // enable update interrupt
    tim.dier.modify(|_, w| w.uie().set_bit());

    // The psc register is buffered, so we trigger an update event to update it
    // Sets the URS bit to prevent an interrupt from being triggered by the UG bit
    tim.cr1.modify(|_, w| w.urs().set_bit());
    tim.egr.write(|w| w.ug().set_bit());
    tim.cr1.modify(|_, w| w.urs().clear_bit());

    tim.cr1.modify(|_, w| {
        w.cms()
            .bits(0b00) // center aligned etc.
            .dir()
            .clear_bit() // upcounting
            .opm()
            .set_bit() // one shot / one pulse
    });
}

fn configure_tim1(tim: &stm32f1xx_hal::pac::TIM1) {
    tim.cnt.write(|w| unsafe { w.bits(0) });

    tim.arr.modify(|_, w| w.arr().bits(0)); // right value in schedule

    tim.psc.modify(|_, w| {
        w.psc().bits(0) // no prescaler
    });

    // enable update interrupt
    tim.dier.modify(|_, w| w.uie().set_bit());

    // The psc register is buffered, so we trigger an update event to update it
    // Sets the URS bit to prevent an interrupt from being triggered by the UG bit
    tim.cr1.modify(|_, w| w.urs().set_bit());
    tim.egr.write(|w| w.ug().set_bit());
    tim.cr1.modify(|_, w| w.urs().clear_bit());

    // start timer
    tim.cr1.modify(|_, w| {
        w.cms()
            .bits(0b00) // center aligned etc.
            .dir()
            .clear_bit() // upcounting
            .opm() // one shot / one pulse
            .enabled()
    });
}

static mut IDX: usize = 0usize;

const START_AT_SCANLINE: usize = 80;
const STOP_AT_SCANLINE: usize = START_AT_SCANLINE + 64 * 3;

#[interrupt]
fn TIM4() {
    unsafe {
        let tim = TIMER_TIM4.as_mut_ptr();
        // clear timer interrupt
        (*tim).sr.modify(|_, w| w.uif().clear_bit());

        let has_pixels = DATA[IDX].has_pixels;
        let new_arr = DATA[IDX].arr;
        let new_crr = DATA[IDX].ccr;
        schedule(new_arr, new_crr);

        if has_pixels && IDX >= START_AT_SCANLINE && IDX < STOP_AT_SCANLINE {
            let mul = (IDX - START_AT_SCANLINE) / 3 * 128;
            draw_pxls(
                (VID_RAM.as_mut_ptr() as *const _ as u32)
                    .overflowing_add(mul as u32)
                    .0,
            );
        }

        IDX += 1;
        if IDX >= DATA.len() {
            IDX = 0;
        }

        let gpioa_set = 0x40010810;
        if IDX >= START_AT_SCANLINE && IDX < STOP_AT_SCANLINE {
            *(gpioa_set as *mut u32) = 1;
        } else {
            *(gpioa_set as *mut u32) = 1 << 16;
        }
    };
}

#[inline(always)]
fn draw_pxls(line_pixel_ptr: u32) {
    unsafe {
        asm!(
            "tim4_busy_loop:",
            "ldrh {cntr},[{tim4_cnt}]",
            "cmp {cntr}, {cntr_dst}",
            "blo tim4_busy_loop",

            "mov {gpio_write_data},#0",
            "loop:",

            "ldrb {gpio_write_data},[{xpxl_data_ptr}]",
            "lsl {gpio_write_data},#3",
            "str {gpio_write_data}, [{gpio_reg}]",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "add {xpxl_data_ptr}, #1",

            "subs {pxl_count}, #1",
            "bne loop",

            "mov {gpio_write_data},#0x0",
            "str {gpio_write_data}, [{gpio_reg}]",

            pxl_count = in(reg) 104, // should be approx 128, probably just 104
            xpxl_data_ptr = in(reg) line_pixel_ptr,
            gpio_write_data = out(reg) _,
            gpio_reg = in(reg) 0x40010c0c,
            cntr = in(reg) 0,
            cntr_dst = in(reg) 1270, // START AT THIS TIM4 CNT VALUE
            tim4_cnt = in(reg) 0x4000_0824 // TIM4 CNT
        );
    }
}

#[interrupt]
fn TIM1_UP() {
    unsafe {
        let tim1 = TIMER_TIM1.as_mut_ptr();
        (*tim1).sr.modify(|_, w| w.uif().clear_bit());
        asm!("wfi");
    }
}

struct Data {
    arr: u16,
    ccr: u16,
    has_pixels: bool,
}

const FULL_SCANLINE_ARR: u16 = 2307 * 2 - ISR_OVERHEAD_CORRECTION; // 64
const HALF_SCANLINE_ARR: u16 = 2305 - ISR_OVERHEAD_CORRECTION;

const BROAD_SYNC_CRR: u16 = 1966 - ISR_OVERHEAD_CORRECTION; // 4.7
const SHORT_SYNC_CRR: u16 = 182 - ISR_OVERHEAD_CORRECTION; // 2.35
const H_SYNC_CRR: u16 = 344 - ISR_OVERHEAD_CORRECTION; // 4.7
                                                       // front porch (after pixel data) 1.64
                                                       // back purch (before pixel data, after h-sync) 5.7

const _DATA: [Data; 4] = [
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    // scanline 6 - 23
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
];

const DATA: [Data; 312 + 5 + 3] = [
    // scanline 1 - 5
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: BROAD_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: BROAD_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: BROAD_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: BROAD_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: BROAD_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    // scanline 6 - 23
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: false,
    },
    // scanline 24 - 309
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    Data {
        arr: FULL_SCANLINE_ARR,
        ccr: H_SYNC_CRR,
        has_pixels: true,
    },
    // scanline 310 - 312
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
    Data {
        arr: HALF_SCANLINE_ARR,
        ccr: SHORT_SYNC_CRR,
        has_pixels: false,
    },
];
