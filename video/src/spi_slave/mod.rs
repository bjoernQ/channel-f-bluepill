use core::{ops::Deref, ptr};

use embedded_hal::spi::{Mode, Phase, Polarity};
use stm32f1xx_hal::{
    gpio::{
        gpioa::{PA4, PA5, PA6, PA7},
        Alternate, Floating, Input, PullDown, PushPull,
    },
    rcc::{Enable, GetBusFreq, Reset},
    spi::{Error, SpiRegisterBlock},
};

pub struct Spi1Slave<SPI> {
    spi: SPI,
    _pins: (
        PA4<Input<PullDown>>,
        PA5<Alternate<PushPull>>,
        PA6<Alternate<PushPull>>,
        PA7<Input<Floating>>,
    ),
}

impl<SPI> Spi1Slave<SPI>
where
    SPI: Deref<Target = SpiRegisterBlock> + Enable + Reset,
{
    /**
      Constructs an SPI instance using SPI1.

      The pin parameter tuple (nss, sck, miso, mosi) should be `(PA4, PA5, PA6, PA7)` configured as `(Input<PullDown>, Alternate<PushPull>, Alternate<PushPull>, Input<Floating>)`.
    */
    pub fn spi1slave(
        spi: SPI,
        pins: (
            PA4<Input<PullDown>>,
            PA5<Alternate<PushPull>>,
            PA6<Alternate<PushPull>>,
            PA7<Input<Floating>>,
        ),
        mode: Mode,
        apb: &mut SPI::Bus,
    ) -> Self
    where
        SPI::Bus: GetBusFreq,
    {
        // enable or reset SPI
        SPI::enable(apb);
        SPI::reset(apb);

        // disable SS output
        spi.cr2.write(|w| w.ssoe().clear_bit());

        spi.cr1.write(|w| {
            w
                // clock phase from config
                .cpha()
                .bit(mode.phase == Phase::CaptureOnSecondTransition)
                // clock polarity from config
                .cpol()
                .bit(mode.polarity == Polarity::IdleHigh)
                // mstr: slave configuration
                .mstr()
                .clear_bit()
                // baudrate not valid for slave
                // lsbfirst: MSB first
                .lsbfirst()
                .clear_bit()
                // ssm: enable hw NSS control
                .ssm()
                .clear_bit()
                // ssi: set nss high = master mode
                .ssi()
                .set_bit()
                // dff: 8 bit frames
                .dff()
                .clear_bit()
                // bidimode: 2-line unidirectional
                .bidimode()
                .clear_bit()
                // both TX and RX are used
                .rxonly()
                .clear_bit()
                // spe: enable the SPI bus
                .spe()
                .set_bit()
        });

        Spi1Slave { spi, _pins: pins }
    }

    pub fn clear_ovr(&self) {
        let _v = unsafe { ptr::read_volatile(&self.spi.dr as *const _ as *const u8) };
        self.spi.sr.read();
    }
}

impl<SPI> embedded_hal::spi::FullDuplex<u8> for Spi1Slave<SPI>
where
    SPI: Deref<Target = SpiRegisterBlock>,
{
    type Error = Error;

    fn read(&mut self) -> nb::Result<u8, Error> {
        let sr = self.spi.sr.read();

        Err(if sr.ovr().bit_is_set() {
            nb::Error::Other(Error::Overrun)
        } else if sr.modf().bit_is_set() {
            nb::Error::Other(Error::ModeFault)
        } else if sr.crcerr().bit_is_set() {
            nb::Error::Other(Error::Crc)
        } else if sr.rxne().bit_is_set() {
            // NOTE(read_volatile) read only 1 byte (the svd2rust API only allows
            // reading a half-word)
            return Ok(unsafe { ptr::read_volatile(&self.spi.dr as *const _ as *const u8) });
        } else {
            nb::Error::WouldBlock
        })
    }

    fn send(&mut self, byte: u8) -> nb::Result<(), Error> {
        let sr = self.spi.sr.read();

        Err(if sr.ovr().bit_is_set() {
            nb::Error::Other(Error::Overrun)
        } else if sr.modf().bit_is_set() {
            nb::Error::Other(Error::ModeFault)
        } else if sr.crcerr().bit_is_set() {
            nb::Error::Other(Error::Crc)
        } else if sr.txe().bit_is_set() {
            // NOTE(write_volatile) see note above
            unsafe { ptr::write_volatile(&self.spi.dr as *const _ as *mut u8, byte) }
            return Ok(());
        } else {
            nb::Error::WouldBlock
        })
    }
}
