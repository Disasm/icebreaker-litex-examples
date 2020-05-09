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
    sk9822.data.write(|w| unsafe {
        w.glob().bits(0x02);
        w.red().bits(0x60);
        w.green().bits(0x20);
        w.blue().bits(0)
    });

    let mut intensity = 0;
    loop {
        // intensity += 1;
        // if intensity > 12 {
        //     intensity = 0;
        // }
        // sk9822.data.write(|w| unsafe {
        //     w.glob().bits(intensity);
        //     w.red().bits(0x60);
        //     w.green().bits(0x20);
        //     w.blue().bits(0)
        // });

        sk9822.control.write(|w| unsafe {
            w.length().bits(12);
            w.start().set_bit()
        });
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
