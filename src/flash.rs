use core::cell::RefCell;

use embassy_rp::flash::{Async, Flash, ERASE_SIZE};
use embassy_rp::peripherals::FLASH;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

use crate::display::messages::DisplayTextMessage;

pub const FLASH_SIZE: usize = 2 * 1024 * 1024;

/// Enum describing flash types and their offset in flash.
#[derive(Clone, Copy)]
pub enum FlashType {
    /// Settings flash.
    Settings = 0x100000,
}

impl FlashType {
    /// Get the offset to know where to start for the FlashType.
    const fn offset(&self) -> u32 {
        *self as u32
    }
}

/// Trait for defining something that is storable in flash.
pub trait FlashStorable<const MAX_SIZE: usize> {
    /// The maximum size of item in flash.
    const MAX_SIZE: usize;

    /// The type of flash it is.
    const FLASH_TYPE: FlashType;

    /// How to serialize the data to store in flash.
    fn serialize(&self) -> &[u8];

    /// How to deserialize the data after reading from flash.
    async fn deserialize(data: &[u8]) -> Self;
}

static FLASH: Mutex<CriticalSectionRawMutex, RefCell<Option<Flash<FLASH, Async, FLASH_SIZE>>>> =
    Mutex::new(RefCell::new(Option::None));

/// Init the flash ref cell.
pub async fn init(flash: Flash<'static, FLASH, Async, FLASH_SIZE>) {
    FLASH.lock().await.replace(Some(flash));
}

/// Write an item into flash memory.
pub async fn write_to_flash<T: FlashStorable<MAX_SIZE>, const MAX_SIZE: usize>(data: &T) {
    let serialized_data = data.serialize();

    // Erase the sector first (adjust size as needed)
    let flash_guard = FLASH.lock().await;

    match flash_guard.borrow_mut().as_mut().unwrap().blocking_erase(
        T::FLASH_TYPE.offset(),
        T::FLASH_TYPE.offset() + 0x00 as u32 + ERASE_SIZE as u32,
    ) {
        Ok(_) => {}
        Err(x) => match x {
            embassy_rp::flash::Error::OutOfBounds => {
                DisplayTextMessage::from_app("Out of bounds", None, None, None)
                    .send_and_replace_queue()
                    .await
            }
            embassy_rp::flash::Error::Unaligned => {
                DisplayTextMessage::from_app("Unaligned", None, None, None)
                    .send_and_replace_queue()
                    .await
            }
            embassy_rp::flash::Error::InvalidCore => {
                DisplayTextMessage::from_app("Invalid core", None, None, None)
                    .send_and_replace_queue()
                    .await
            }
            embassy_rp::flash::Error::Other => {
                DisplayTextMessage::from_app("Other", None, None, None)
                    .send_and_replace_queue()
                    .await
            }
        },
    }

    flash_guard
        .borrow_mut()
        .as_mut()
        .unwrap()
        .blocking_write(T::FLASH_TYPE.offset(), serialized_data)
        .unwrap();
}

/// Read an item from flash memory.
pub async fn read_from_flash<T: FlashStorable<MAX_SIZE>, const MAX_SIZE: usize>() -> T {
    let mut buffer = [0u8; MAX_SIZE];

    let flash_guard = FLASH.lock().await;

    flash_guard
        .borrow_mut()
        .as_mut()
        .unwrap()
        .read(T::FLASH_TYPE.offset(), &mut buffer)
        .await
        .unwrap();

    drop(flash_guard);

    T::deserialize(&buffer).await
}
