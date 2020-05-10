#![no_std]
#![no_main]

extern crate panic_halt;

use icebesoc_pac;
use riscv_rt::entry;

mod timer;
mod leds;
mod print;

use timer::Timer;
use leds::Leds;

const SYSTEM_CLOCK_FREQUENCY: u32 = 21_000_000;

// This is the entry point for the application.
// It is not allowed to return.
#[entry]
fn main() -> ! {
    let peripherals = icebesoc_pac::Peripherals::take().unwrap();

    print::print_hardware::set_hardware(peripherals.UART);
    let mut timer = Timer::new(peripherals.TIMER0);
    let mut leds = Leds::new(peripherals.LEDS);
    leds.set_single(true, false, true, false, true, false, true);

    let sk9822 = peripherals.SK9822;

    static mut DATA: [u32; 14] = [0xe0000000; 14];
    unsafe {
        DATA[0] = 0;
        DATA[13] = 0xffffffff;
    }
    sk9822.address.write(|w| unsafe {
        w.bits(DATA.as_ptr() as usize as u32)
    });

    let mut index = 0;
    loop {
        unsafe {
            for i in 0..12 {
                let mut i2 = i + index;
                if i2 >= 12 {
                    i2 -= 12;
                }
                if i2 > 4 {
                    i2 = 0;
                }

                DATA[i+1] = 0xe0002060 | ((i2 as u32) << 24);

                // if i == index {
                //     DATA[i+1] = 0xe2002060;
                // } else {
                //     DATA[i+1] = 0xe0000000;
                // }
            }
        }
        index += 1;
        if index >= 12 {
            index = 0;
        }

        sk9822.control.write(|w| unsafe {
            w.length().bits(14);
            w.start().set_bit()
        });
        while sk9822.status.read().busy().bit_is_set() { }
        print!("a");
        leds.toggle();
        msleep(&mut timer, 160);
    }
}

fn msleep(timer: &mut Timer, ms: u32) {
    timer.disable();

    timer.reload(0);
    timer.load(SYSTEM_CLOCK_FREQUENCY / 1_000 * ms);

    timer.enable();

    // Wait until the time has elapsed
    while timer.value() > 0 {}
}
