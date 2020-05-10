from litex.soc.interconnect import wishbone
from litex.soc.interconnect.csr import *
from litex.soc.integration.doc import AutoDoc, ModuleDoc


class SK9822(Module, AutoCSR, AutoDoc):
    """SK9822 driver.
    """
    def __init__(self, pad_clock, pad_data):
        # Documentation
        self.intro = ModuleDoc("""SK9822 driver""")

        self._control = CSRStorage(fields=[
            CSRField("start", size=1, offset=0, pulse=True, description="Write ``1`` to start transfer"),
            CSRField("length", size=8, offset=8, description="Number of LEDs")
        ], description="SK9822 control")

        self._address = CSRStorage(32, description="Memory address")

        self._status = CSRStatus(fields=[
            CSRField("busy", size=1, offset=0, description="Transfer is in progress")
        ], description="SK9822 status")

        self.wishbone = wishbone.Interface()

        self.start = Signal()
        self.data = Signal(32)
        self.bit_counter = Signal(5)
        self.dword_counter = Signal(8)
        self.idle = Signal()
        self.sender_idle = Signal()
        self.comb += [
            self.start.eq(self._control.fields.start),
            self._status.fields.busy.eq(~self.idle),
        ]

        self.fetch_offset = Signal(8)
        self.start_fetch = Signal()
        self.fetched_data = Signal(32)
        self.comb += [
            self.wishbone.adr.eq((self._address.storage >> 2) + self.fetch_offset),
            self.wishbone.sel.eq(2 ** len(self.wishbone.sel) - 1)
        ]
        self.submodules.wb_fsm = wb_fsm = FSM(reset_state="IDLE")
        wb_fsm.act("IDLE",
            If(self.start_fetch,
                NextState("READ"),
            )
        )
        wb_fsm.act("READ",
            self.wishbone.stb.eq(1),
            self.wishbone.we.eq(0),
            self.wishbone.cyc.eq(1),
            If(self.wishbone.ack,
                NextState("IDLE"),
                NextValue(self.fetched_data, self.wishbone.dat_r),
                NextValue(self.start_fetch, 0),
            )
        )

        self.submodules.fsm = fsm = FSM(reset_state="IDLE")
        fsm.act("IDLE",
            self.idle.eq(1),
            self.sender_idle.eq(1),
            If(self.start,
                NextState("WAIT"),
                NextValue(self.fetch_offset, 0),
                NextValue(self.start_fetch, 1),
            )
        )
        fsm.act("WAIT",
            self.sender_idle.eq(1),
            If(self.start_fetch == 0,
                NextState("SEND"),
                NextValue(self.data, self.fetched_data),
                NextValue(self.dword_counter, self._control.fields.length),
                NextValue(self.bit_counter, 31),
            )
        )
        fsm.act("SEND",
            If(self.dword_counter == 0,
                self.sender_idle.eq(1),
                NextState("IDLE"),
            ).Elif(self.bit_counter == 0,
                NextValue(self.data, self.fetched_data),
                NextValue(self.dword_counter, self.dword_counter - 1)
            ).Elif(self.bit_counter == 31,
                NextValue(self.fetch_offset, self.fetch_offset + 1),
                NextValue(self.start_fetch, 1),
            )
        )

        self.sync += [
            If(~self.sender_idle,
                self.data.eq(self.data << 1),
                self.bit_counter.eq(self.bit_counter - 1),
            )
        ]

        self.comb += [
            pad_clock.eq(~ClockSignal() & ~self.sender_idle),
            pad_data.eq(self.data[31]),
        ]
