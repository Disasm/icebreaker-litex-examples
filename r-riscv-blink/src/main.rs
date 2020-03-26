#![no_std]
#![no_main]

extern crate panic_halt;

use icebesoc_pac;
use riscv_rt::entry;

mod timer;
mod leds;
mod print;
mod usb_eptri;

use timer::Timer;
use leds::Leds;
use usb_eptri::Usb;
use usb_device::prelude::*;

const SYSTEM_CLOCK_FREQUENCY: u32 = 12_000_000;

// This is the entry point for the application.
// It is not allowed to return.
#[entry]
fn main() -> ! {
    let peripherals = icebesoc_pac::Peripherals::take().unwrap();

    print::print_hardware::set_hardware(peripherals.UART);
    let mut timer = Timer::new(peripherals.TIMER0);
    let mut leds = Leds::new(peripherals.LEDS);
    leds.off();

    println!("starting...");

    let usb = peripherals.USB;

    // Disconnect from bus
    usb.pullup_out.write(|w| w.pullup_out().clear_bit());
    msleep(&mut timer, 100);

    let usb_bus = Usb::new(usb);

    let mut usb_dev = UsbDeviceBuilder::new(usb_bus, UsbVidPid(0x16c0, 0x27dd))
        .manufacturer("Fake company")
        .product("Enumeration test")
        .serial_number("iCEBreaker")
        .build();

    loop {
        if usb_dev.poll(&mut []) {
            leds.toggle_mask(0b10);
        }
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
