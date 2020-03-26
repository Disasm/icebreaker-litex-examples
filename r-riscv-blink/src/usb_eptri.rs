use icebesoc_pac::USB;
use usb_device::prelude::*;
use usb_device::bus::{UsbBus, PollResult, EndpointAllocator};
use usb_device::endpoint::{EndpointAddress, EndpointIn, Endpoint, EndpointDescriptor, EndpointOut, EndpointType};
use usb_device::allocator::{UsbAllocator, EndpointConfig};
use usb_device::{Result, UsbDirection};
use crate::{print, println};

fn set_stalled(ep_addr: EndpointAddress, stalled: bool) {
    //println!("set_stalled: {:?} (dir {:?}) is? {}", stalled, ep_addr.direction(), is_stalled(ep_addr));

    let usb = unsafe { &*USB::ptr() };
    //println!("enables before: {:04x}", usb.out_enable_status.read().bits());

    match ep_addr.direction() {
        UsbDirection::Out => {
            usb.out_ctrl.write(|w| unsafe {
                // OUT endpoint is un-stalled automatically with SETUP request.
                // However, it may remain in disabled state after that, so we
                // should re-enable it.

                w.epno().bits(ep_addr.number());
                if stalled {
                    w.stall().set_bit()
                } else {
                    w.stall().clear_bit();
                    w.enable().set_bit()
                }
            });
        },
        UsbDirection::In => {
            if is_stalled(ep_addr) == stalled {
                return;
            }
            usb.in_ctrl.write(|w| unsafe {
                w.epno().bits(ep_addr.number());
                if stalled {
                    w.stall().set_bit()
                } else {
                    w.stall().clear_bit();

                    // Writing an empty request will send an empty data packet.
                    // So we reset the endpoint instead.
                    w.reset().set_bit()
                }
            });
        },
    }
    //println!("enables: {:04x}", usb.out_enable_status.read().bits());
}

fn is_stalled(ep_addr: EndpointAddress) -> bool {
    let usb = unsafe { &*USB::ptr() };

    let mask = (1 << ep_addr.number()) as u16;
    let stall_status = match ep_addr.direction() {
        UsbDirection::Out => usb.out_stall_status.read().out_stall_status().bits(),
        UsbDirection::In => usb.in_stall_status.read().in_stall_status().bits(),
    };

    stall_status & mask != 0
}

pub struct Usb {
    usb: USB
}

impl Usb {
    /// Constructs a new USB peripheral driver.
    pub fn new(usb: USB) -> UsbAllocator<Self> {
        unsafe {
            usb.pullup_out.write(|w| w.pullup_out().clear_bit());
            usb.address.write(|w| w.addr().bits(0));
            usb.out_ctrl.write(|w| w.bits(0));

            usb.setup_ev_enable.write(|w| w.bits(0));
            usb.in_ev_enable.write(|w| w.bits(0));
            usb.out_ev_enable.write(|w| w.bits(0));

            usb.in_ctrl.write(|w| w.reset().set_bit());
            usb.setup_ctrl.write(|w| w.reset().set_bit());
            usb.out_ctrl.write(|w| w.reset().set_bit());
        }


        let bus = Usb {
            usb
        };

        UsbAllocator::new(bus)
    }
}

impl UsbBus for Usb {
    type EndpointOut = EptriEndpoint;
    type EndpointIn = EptriEndpoint;
    type EndpointAllocator = EptriAllocator;

    fn create_allocator(&mut self) -> Self::EndpointAllocator {
        EptriAllocator::new()
    }

    fn enable(&mut self) {
        self.usb.pullup_out.write(|w| w.pullup_out().set_bit());

        // Clear events
        self.usb.setup_ev_pending.modify(|_, w| w);
        self.usb.in_ev_pending.modify(|_, w| w);
        self.usb.out_ev_pending.modify(|_, w| w);

        // Subscribe to all the events
        unsafe {
            self.usb.setup_ev_enable.write(|w| w.bits(0b11));
            self.usb.in_ev_enable.write(|w| w.bits(0b1));
            self.usb.out_ev_enable.write(|w| w.bits(0b1));
        }

        // Reset handlers
        self.usb.in_ctrl.write(|w| w.reset().set_bit());
        self.usb.setup_ctrl.write(|w| w.reset().set_bit());
        self.usb.out_ctrl.write(|w| w.reset().set_bit());
    }

    fn reset(&mut self) {
        //println!("reset()");

        // Clear events
        self.usb.setup_ev_pending.modify(|_, w| w);
        self.usb.in_ev_pending.modify(|_, w| w);
        self.usb.out_ev_pending.modify(|_, w| w);

        // Reset handlers
        self.usb.in_ctrl.write(|w| w.reset().set_bit());
        self.usb.setup_ctrl.write(|w| w.reset().set_bit());
        self.usb.out_ctrl.write(|w| w.reset().set_bit());

        self.usb.address.write(|w| unsafe { w.addr().bits(0) });
    }

    fn poll(&mut self) -> PollResult {
        let setup_ev = self.usb.setup_ev_pending.read().setup_ev_pending().bits();
        if setup_ev & 0b10 != 0 {
            // Clear `reset` flag
            self.usb.setup_ev_pending.write(|w| unsafe { w.bits(0b10) });

            //println!("reset");

            return PollResult::Reset;
        }

        let mut ep_setup = if setup_ev & 0b01 != 0 {
            let epno = self.usb.setup_status.read().epno().bits();
            (1 << epno) as u16
        } else {
            0
        };

        let out_ev = self.usb.out_ev_pending.read().out_ev_pending().bits();
        let mut ep_out = if out_ev {
            let epno = self.usb.out_status.read().epno().bits();
            (1 << epno) as u16
        } else {
            0
        };

        let in_ev = self.usb.in_ev_pending.read().in_ev_pending().bits();
        let ep_in_complete = if in_ev {
            // Clear `packet` flag
            self.usb.in_ev_pending.write(|w| unsafe { w.bits(1) });

            // Assume that `in_ctrl` holds the last `epno` used to send data
            let epno = self.usb.in_ctrl.read().epno().bits();
            (1 << epno) as u16
        } else {
            0
        };

        if ep_out | ep_in_complete | ep_setup != 0 {
            //println!("out={}, in={}, setup={}", ep_out, ep_in_complete, ep_setup);

            // Report only one EP0 event at the same time
            if (ep_out | ep_in_complete | ep_setup) & 1 != 0 {
                if ep_in_complete & 1 != 0 {
                    // in_complete event always arrives first
                    // Ignore all the other events.
                    ep_out &= !1;
                    ep_setup &= !1;

                } else if ep_out & ep_setup & 1 != 0 {
                    // We have both SETUP and OUT events
                    // Well, process OUT first.

                    // This assumption can be wrong when SETUP + OUT arrive in succession,
                    // But there is no way to detect this case.

                    ep_setup &= !1;
                }
            }

            return PollResult::Data {
                ep_out,
                ep_in_complete,
                ep_setup
            };
        }

        PollResult::None
    }

    fn set_device_address(&mut self, addr: u8) {
        //println!("addr: {}", addr);
        self.usb.address.write(|w| unsafe { w.addr().bits(addr) });
    }

    fn set_stalled(&mut self, ep_addr: EndpointAddress, stalled: bool) {
        set_stalled(ep_addr, stalled)
    }

    fn is_stalled(&self, ep_addr: EndpointAddress) -> bool {
        is_stalled(ep_addr)
    }

    fn suspend(&mut self) {
        // Do nothing
    }

    fn resume(&mut self) {
        // Do nothing
    }
}

pub struct EptriEndpoint {
    descr: EndpointDescriptor,
}

impl Endpoint for EptriEndpoint {
    fn descriptor(&self) -> &EndpointDescriptor {
        &self.descr
    }

    fn enable(&mut self) {
        let usb = unsafe { &*USB::ptr() };

        if self.ep_type() == EndpointType::Control {
            usb.setup_ctrl.write(|w| w.reset().set_bit());
        }

        match self.descr.address.direction() {
            UsbDirection::Out => {
                usb.out_ctrl.write(|w| unsafe {
                    w.epno().bits(self.descr.address.number());
                    w.reset().set_bit()
                });
                usb.out_ctrl.write(|w| unsafe {
                    w.epno().bits(self.descr.address.number());
                    w.enable().set_bit()
                });
            },
            UsbDirection::In => {
                usb.in_ctrl.write(|w| unsafe {
                    w.epno().bits(self.descr.address.number());
                    w.reset().set_bit()
                });
            },
        }
    }

    fn disable(&mut self) {
        let usb = unsafe { &*USB::ptr() };

        match self.descr.address.direction() {
            UsbDirection::Out => {
                usb.out_ctrl.write(|w| unsafe {
                    w.epno().bits(self.descr.address.number());
                    w.reset().set_bit()
                });
            },
            UsbDirection::In => {
                usb.in_ctrl.write(|w| unsafe {
                    w.epno().bits(self.descr.address.number());
                    w.reset().set_bit()
                });
            },
        }
    }

    fn set_stalled(&mut self, stalled: bool) {
        set_stalled(self.descr.address, stalled)
    }

    fn is_stalled(&self) -> bool {
        is_stalled(self.descr.address)
    }
}

impl EndpointIn for EptriEndpoint {
    fn write(&mut self, data: &[u8]) -> Result<()> {
        let usb = unsafe { &*USB::ptr() };

        if usb.in_status.read().idle().bit_is_set() {
            if data.len() > (self.descr.max_packet_size as usize) {
                return Err(UsbError::BufferOverflow)
            }

            for b in data {
                usb.in_data.write(|w| unsafe { w.data().bits(*b) });
            }

            // println!("write in: {}", data.len());
            //
            // if data.len() > 0 {
            //     for b in data {
            //         print!("{:02x} ", *b);
            //     }
            //     println!();
            // }

            // Arm the endpoint
            usb.in_ctrl.write(|w| unsafe { w.epno().bits(self.descr.address.number()) });

            Ok(())
        } else {
            Err(UsbError::WouldBlock)
        }
    }
}

impl EndpointOut for EptriEndpoint {
    fn read(&mut self, data: &mut [u8]) -> Result<usize> {
        let usb = unsafe { &*USB::ptr() };

        let status = usb.out_status.read();
        if status.have().bit_is_set() && status.epno().bits() == self.descr.address.number() {
            let mut buf = [0; 66];
            let mut len = 0;

            while usb.out_status.read().have().bit_is_set() && len < buf.len() {
                buf[len] = usb.out_data.read().data().bits();
                len += 1;
            }

            //println!("read out: {}", len-2);

            // Clear `done` flag
            usb.out_ev_pending.write(|w| unsafe { w.bits(1) });

            // Re-enable endpoint
            usb.out_ctrl.write(|w| unsafe {
                w.epno().bits(self.descr.address.number());
                w.enable().set_bit()
            });

            if len < 2 {
                // In fact, this should be panic
                return Err(UsbError::WouldBlock);
            }

            len = len - 2;
            if len > data.len() {
                return Err(UsbError::BufferOverflow);
            }

            // if len > 0 {
            //     for b in &buf[..len] {
            //         print!("{:02x} ", *b);
            //     }
            //     println!();
            // }

            data[..len].copy_from_slice(&buf[..len]);
            return Ok(len);
        }

        let status = usb.setup_status.read();
        if status.have().bit_is_set() && status.epno().bits() == self.descr.address.number() {
            let mut buf = [0; 10];
            let mut len = 0;

            while usb.setup_status.read().have().bit_is_set() && len < buf.len() {
                buf[len] = usb.setup_data.read().data().bits();
                len += 1;
            }

            //println!("read setup: {}", len - 2);

            usb.setup_ev_pending.write(|w| unsafe { w.bits(0b01) }); // Clear `packet` flag

            if len < 2 {
                // In fact, this should be panic
                return Err(UsbError::WouldBlock);
            }

            len = len - 2;
            if len > data.len() {
                return Err(UsbError::BufferOverflow);
            }

            // if len > 0 {
            //     for b in &buf[..len] {
            //         print!("{:02x} ", *b);
            //     }
            //     println!();
            // }

            data[..len].copy_from_slice(&buf[..len]);
            Ok(len)
        } else {
            Err(UsbError::WouldBlock)
        }
    }
}

pub struct EptriAllocator {
    in_mask: u16,
    out_mask: u16,
}

impl EptriAllocator {
    pub fn new() -> EptriAllocator {
        EptriAllocator {
            in_mask: 0,
            out_mask: 0
        }
    }

    fn alloc_number(ep_mask: &mut u16, config: &EndpointConfig) -> Result<u8> {
        if let Some(number) = config.number {
            let mask = (1 << number) as u16;
            if *ep_mask & mask != 0 {
                return Err(UsbError::InvalidEndpoint);
            }
            *ep_mask |= mask;
            Ok(number)
        } else {
            for i in 1..16 {
                let mask = (1 << i) as u16;
                if *ep_mask & mask == 0 {
                    *ep_mask |= mask;
                    return Ok(i as u8)
                }
            }
            Err(UsbError::EndpointOverflow)
        }
    }

    fn alloc_ep(ep_mask: &mut u16, direction: UsbDirection, config: &EndpointConfig) -> Result<EptriEndpoint> {
        if config.max_packet_size > 64 {
            return Err(UsbError::Unsupported);
        }
        if config.ep_type == EndpointType::Isochronous {
            return Err(UsbError::Unsupported);
        }
        let number = Self::alloc_number(ep_mask, config)?;
        let descr = EndpointDescriptor {
            address: EndpointAddress::from_parts(number, direction),
            ep_type: config.ep_type,
            max_packet_size: config.max_packet_size,
            interval: config.interval
        };
        Ok(EptriEndpoint {
            descr
        })
    }
}

impl EndpointAllocator<Usb> for EptriAllocator {
    fn alloc_out(&mut self, config: &EndpointConfig) -> Result<EptriEndpoint> {
        Self::alloc_ep(&mut self.out_mask, UsbDirection::Out, config)
    }

    fn alloc_in(&mut self, config: &EndpointConfig) -> Result<EptriEndpoint> {
        Self::alloc_ep(&mut self.in_mask, UsbDirection::In, config)
    }
}
