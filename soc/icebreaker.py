#!/usr/bin/env python3

# This file is Copyright (c) 2019 Sean Cross <sean@xobs.io>
# This file is Copyright (c) 2018 David Shah <dave@ds0.me>
# This file is Copyright (c) 2020 Piotr Esden-Tempski <piotr@esden.net>
# License: BSD

# This target was originally based on the Fomu target.

# This variable defines all the external programs that this module
# relies on.  lxbuildenv reads this variable in order to ensure
# the build will finish without exiting due to missing third-party
# programs.
LX_DEPENDENCIES = ["riscv", "icestorm", "yosys", "nextpnr-ice40"]

# Import lxbuildenv to integrate the deps/ directory
import lxbuildenv

import os
import argparse

from migen import *
from migen.genlib.resetsync import AsyncResetSynchronizer

from litex.soc.cores import up5kspram, spi_flash
from litex.soc.integration.soc_core import SoCCore
from litex.soc.integration.builder import Builder, builder_argdict, builder_args
from litex.soc.integration.soc_core import soc_core_argdict, soc_core_args
from litex.soc.integration.doc import AutoDoc

from litex_boards.platforms.icebreaker import Platform

from litex.soc.interconnect import wishbone
from litex.soc.cores.uart import UARTWishboneBridge
from rtl.leds import Leds

import litex.soc.doc as lxsocdoc
from litex.soc.integration import export
from litex.build.tools import write_to_file


class JumpToAddressROM(wishbone.SRAM):
    def __init__(self, size, addr):
        data = [
            0x00000537 | ((addr & 0xfffff000) << 0),   # lui   a0,%hi(addr)
            0x00052503 | ((addr & 0x00000fff) << 20),  # lw    a0,%lo(addr)(a0)
            0x000500e7,                                # jalr  a0
        ]
        wishbone.SRAM.__init__(self, size, read_only=True, init=data)

# CRG ----------------------------------------------------------------------------------------------


class _CRG(Module, AutoDoc):
    """Icebreaker Clock Resource Generator

    The whole system is clocked by the external 12MHz clock.
    """
    def __init__(self, platform):
        self.clock_domains.cd_sys = ClockDomain()
        self.clock_domains.cd_por = ClockDomain()
        self.reset = Signal()

        # # #
        reset_delay = Signal(12, reset=4095)

        # Clocks
        clk12 = platform.request("clk12")
        platform.add_period_constraint(clk12, 1e9 / 12e6)
        self.comb += self.cd_sys.clk.eq(clk12)
        self.comb += self.cd_por.clk.eq(clk12)
        self.comb += self.cd_sys.rst.eq(reset_delay != 0)

        # Power On Reset
        self.sync.por += If(reset_delay != 0, reset_delay.eq(reset_delay - 1))
        self.specials += AsyncResetSynchronizer(self.cd_por, self.reset)


# BaseSoC ------------------------------------------------------------------------------------------

class BaseSoC(SoCCore):
    """A SoC on iCEBreaker, optionally with a softcore CPU"""

    # Statically-define the memory map, to prevent it from shifting across various litex versions.
    SoCCore.mem_map = {
        "rom":              0x00000000,  # (default shadow @0x80000000)
        "sram":             0x10000000,  # (default shadow @0xa0000000)
        "spiflash":         0x20000000,  # (default shadow @0xa0000000)
        "csr":              0xe0000000,  # (default shadow @0x60000000)
        "vexriscv_debug":   0xf00f0000,
    }

    def __init__(self, debug=True, boot_vector=0x20040000, **kwargs):
        """Create a basic SoC for iCEBreaker.

        Create a basic SoC for iCEBreaker.  The `sys` frequency will run at 12 MHz.

        Returns:
            Newly-constructed SoC
        """
        platform = Platform()

        kwargs["cpu_variant"]       = "lite"
        kwargs["cpu_reset_address"] = boot_vector
        if debug:
            kwargs["uart_name"]   = "crossover"
            kwargs["cpu_variant"] = "lite+debug"

        clk_freq = int(12e6)

        # Force the SRAM size to 0, because we add our own SRAM with SPRAM
        kwargs["integrated_sram_size"] = 0
        kwargs["integrated_rom_size"]  = 0

        kwargs["csr_data_width"] = 32

        SoCCore.__init__(self, platform, clk_freq, **kwargs)

        # If there is a VexRiscv CPU, add a fake ROM that simply tells the CPU
        # to jump to the given address.
        if hasattr(self, "cpu") and self.cpu.name == "vexriscv":
            self.add_memory_region("rom", 0, 16)
            self.submodules.rom = JumpToAddressROM(16, boot_vector)

        self.submodules.crg = _CRG(platform)

        # UP5K has single port RAM, which is a dedicated 128 kilobyte block.
        # Use this as CPU RAM.
        spram_size = 128 * 1024
        self.submodules.spram = up5kspram.Up5kSPRAM(size=spram_size)
        self.register_mem("sram", self.mem_map["sram"], self.spram.bus, spram_size)

        # The litex SPI module supports memory-mapped reads, as well as a bit-banged mode
        # for doing writes.
        spi_pads = platform.request("spiflash4x")
        self.submodules.lxspi = spi_flash.SpiFlash(spi_pads, dummy=6, endianness="little")
        self.register_mem("spiflash", self.mem_map["spiflash"], self.lxspi.bus, size=16 * 1024 * 1024)
        self.add_csr("lxspi")

        # In debug mode, add a UART bridge.  This takes over from the normal UART bridge,
        # however you can use the "crossover" UART to communicate with this over the bridge.
        if debug:
            self.submodules.uart_bridge = UARTWishboneBridge(platform.request("serial"), clk_freq, baudrate=115200)
            self.add_wb_master(self.uart_bridge.wishbone)
            if hasattr(self, "cpu") and self.cpu.name == "vexriscv":
                self.register_mem("vexriscv_debug", 0xf00f0000, self.cpu.debug_bus, 0x100)

        self.submodules.leds = Leds(Cat(
            platform.request("user_ledr_n"),
            platform.request("user_ledg_n")),
            led_polarity=0x03,
            led_name=[
                ["ledr", "The Red LED on the main iCEBreaker board."],
                ["ledg", "The Green LED on the main iCEBreaker board."]])

        self.add_csr("leds")

    def set_yosys_nextpnr_settings(self, nextpnr_seed=0, nextpnr_placer="heap"):
        """Set Yosys/Nextpnr settings by overriding default LiteX's settings.
        Args:
            nextpnr_seed   (int): Seed to use in Nextpnr
            nextpnr_placer (str): Placer to use in Nextpnr
        """
        assert hasattr(self.platform.toolchain, "yosys_template")
        assert hasattr(self.platform.toolchain, "build_template")
        self.platform.toolchain.yosys_template = [
            "{read_files}",
            "attrmap -tocase keep -imap keep=\"true\" keep=1 -imap keep=\"false\" keep=0 -remove keep=0",
            # Use "-relut -dffe_min_ce_use 4" to the synth_ice40 command. The "-reult" adds an additional
            # LUT pass to pack more stuff in, and the "-dffe_min_ce_use 4" flag prevents Yosys from
            # generating a Clock Enable signal for a LUT that has fewer than 4 flip-flops. This increases
            # density, and lets us use the FPGA more efficiently.
            "synth_ice40 -json {build_name}.json -top {build_name} -relut -abc2 -dffe_min_ce_use 4 -relut",
        ]
        self.platform.toolchain.build_template = [
            "yosys -q -l {build_name}.rpt {build_name}.ys",
            "nextpnr-ice40 --json {build_name}.json --pcf {build_name}.pcf --asc {build_name}.txt" +
            " --pre-pack {build_name}_pre_pack.py --{architecture} --package {package}" +
            " --seed {}".format(nextpnr_seed) +
            " --placer {}".format(nextpnr_placer),
            # Disable final deep-sleep power down so firmware words are loaded onto softcore's address bus.
            "icepack -s {build_name}.txt {build_name}.bin"
        ]

    def generate_memory_x(self, destination):
        write_to_file(os.path.join(destination, "memory.x"), get_memory_x(self.mem_regions, self.cpu.reset_address))


def get_memory_x(mem_regions, cpu_reset_address):
    r = export.get_linker_regions(mem_regions)
    r += '\n'
    r += 'REGION_ALIAS("REGION_TEXT", spiflash);\n'
    r += 'REGION_ALIAS("REGION_RODATA", spiflash);\n'
    r += 'REGION_ALIAS("REGION_DATA", sram);\n'
    r += 'REGION_ALIAS("REGION_BSS", sram);\n'
    r += 'REGION_ALIAS("REGION_HEAP", sram);\n'
    r += 'REGION_ALIAS("REGION_STACK", sram);\n\n'
    r += '/* CPU reset location. */\n'
    r += '_stext = {:#08x};\n'.format(cpu_reset_address)
    return r


# Build --------------------------------------------------------------------------------------------
def main():
    parser = argparse.ArgumentParser(description="LiteX SoC on iCEBreaker")
    parser.add_argument("--nextpnr-seed", default=0, help="Select nextpnr pseudo random seed")
    parser.add_argument("--nextpnr-placer", default="heap", choices=["sa", "heap"], help="Select nextpnr placer algorithm")
    parser.add_argument("--debug", action="store_true", help="Enable debug features. (UART has to be used with the wishbone-tool.)")
    parser.add_argument("--document-only", action="store_true", help="Do not build a soc. Only generate documentation.")
    builder_args(parser)
    soc_core_args(parser)
    args = parser.parse_args()

    soc = BaseSoC(debug=args.debug, **soc_core_argdict(args))
    soc.set_yosys_nextpnr_settings(nextpnr_seed=args.nextpnr_seed, nextpnr_placer=args.nextpnr_placer)

    # Don't build software -- we don't include it since we just jump to SPI flash.
    builder_kwargs = builder_argdict(args)
    builder_kwargs["compile_software"] = False
    if args.document_only:
        builder_kwargs["compile_gateware"] = False
    if builder_kwargs["csr_svd"] is None:
        builder_kwargs["csr_svd"] = "../rust/icebesoc-pac/iCEBESOC.svd"
    builder = Builder(soc, **builder_kwargs)
    builder.build()
    soc.generate_memory_x("../rust/icebesoc-pac/")
    lxsocdoc.generate_docs(soc, "build/documentation/", project_name="iCEBreaker LiteX Riscv Example SOC", author="Piotr Esden-Tempski")


if __name__ == "__main__":
    main()
