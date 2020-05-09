from litex.soc.interconnect.csr import *
from litex.soc.integration.doc import AutoDoc, ModuleDoc


class SK9822(Module, AutoCSR, AutoDoc):
    """SK9822 driver.

    Attributes:
        led_pin: Signals of the LED pin outputs.
        led_polarity: Bit pattern to adjust polarity. 0 stays the same 1 inverts the signal.
        led_name: Array of the LED names and descriptions. [["name1", "description1"], ["name2", "description2"]]
    """
    def __init__(self, pad_clock, pad_data):
        # Documentation
        self.intro = ModuleDoc("""SK9822 driver.
        The LEDs are inverted as these are negative logic LED. This means that if you set the
        corresponding LED bit to 1 the LED will be off and if you set it to 0 the LED will be on.
        """)

        # HDL Implementationj
        # self._out = CSRStorage(len(led_pin), fields=[
        #     CSRField(fld[0], description=fld[1]) for fld in led_name
        # ])
        # self.comb += led_pin.eq(self._out.storage ^ led_polarity)
        self._control = CSRStorage(fields=[
            CSRField("start", size=1, offset=0, pulse=True, description="Write ``1`` to start transfer"),
            CSRField("length", size=8, offset=8, description="Number of LEDs")
        ], description="SK9822 control")

        self._status = CSRStatus(fields=[
            CSRField("busy", size=1, offset=0, description="Transfer is in progress")
        ], description="SK9822 status")

        self._data = CSRStorage(fields=[
            CSRField("red", size=8, offset=0, description="Red intensity"),
            CSRField("green", size=8, offset=8, description="Green intensity"),
            CSRField("blue", size=8, offset=16, description="Blue intensity"),
            CSRField("glob", size=5, offset=24, description="Global intensity"),
        ], description="Pixel data")

        self.start = Signal()
        self.data = Signal(32)
        self.bit_counter = Signal(5)
        self.dword_counter = Signal(8)
        self.busy = Signal()
        self.comb += [
            self.start.eq(self._control.fields.start),
            self._status.fields.busy.eq(self.busy),
        ]

        self.submodules.fsm = fsm = FSM(reset_state="IDLE")
        fsm.act("IDLE",
            self.busy.eq(0),
            If(self.start,
                NextState("START"),
                NextValue(self.data, Constant(0, 32)),
                NextValue(self.bit_counter, 31)
            )
        )
        fsm.act("START",
            self.busy.eq(1),
            If(self.bit_counter == 0,
                NextState("DATA"),
                NextValue(self.data, Cat(
                    self._data.fields.red,
                    self._data.fields.green,
                    self._data.fields.blue,
                    self._data.fields.glob,
                    Constant(0b111)
                )),
                NextValue(self.dword_counter, self._control.fields.length),
            )
        )
        fsm.act("DATA",
            self.busy.eq(1),
            If(self.dword_counter == 0,
                NextState("END"),
                NextValue(self.data, Constant(0xffffffff, 32)),
            ).Elif(self.bit_counter == 0,
                NextValue(self.data, Cat(
                    self._data.fields.red,
                    self._data.fields.green,
                    self._data.fields.blue,
                    self._data.fields.glob,
                    Constant(0b111)
                )),
                NextValue(self.dword_counter, self.dword_counter - 1)
            )
        )
        fsm.act("END",
            self.busy.eq(1),
            If(self.bit_counter == 0,
                NextState("IDLE"),
            )
        )

        self.sync += [
            If(self.busy == 1,
                self.data.eq(self.data << 1),
                self.bit_counter.eq(self.bit_counter - 1),
            )
        ]

        self.comb += [
            pad_clock.eq(~ClockSignal() & self.busy),
            pad_data.eq(self.data[31]),
        ]
