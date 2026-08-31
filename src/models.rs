use std::os::raw::c_char;

// ==========================================
// 1. ADD ORDER MESSAGES
// ==========================================

/// Message Type 'A' - Add Order (Anonymous)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ItchAddOrder {
    pub message_type: c_char,       // 'A'
    pub stock_locate: u16,          // Numeric ticker identifier
    pub tracking_number: u16,       // Internal tracking
    pub timestamp_bytes: [u8; 6],   // Nanoseconds since midnight (48-bit)
    pub order_reference: u64,       // Unique order ID
    pub buy_sell_indicator: c_char, // 'B' = Buy, 'S' = Sell
    pub shares: u32,                // Quantity
    pub stock: [u8; 8],            // Right-padded ASCII symbol
    pub price: u32,                 // Price scaled by 10,000
}

/// Message Type 'F' - Add Order (Attributed / Shows MPID)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ItchAddOrderAttributed {
    pub message_type: c_char,       // 'F'
    pub stock_locate: u16,          // Numeric ticker identifier
    pub tracking_number: u16,       // Internal tracking
    pub timestamp_bytes: [u8; 6],   // Nanoseconds since midnight (48-bit)
    pub order_reference: u64,       // Unique order ID
    pub buy_sell_indicator: c_char, // 'B' = Buy, 'S' = Sell
    pub shares: u32,                // Quantity
    pub stock: [u8; 8],            // Right-padded ASCII symbol
    pub price: u32,                 // Price scaled by 10,000
    pub attribution: [u8; 4],       // Market Participant Identifier (MPID)
}

// ==========================================
// 2. ORDER EXECUTION MESSAGES
// ==========================================

/// Message Type 'E' - Order Executed
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ItchOrderExecuted {
    pub message_type: c_char,       // 'E'
    pub stock_locate: u16,          // Numeric ticker identifier
    pub tracking_number: u16,       // Internal tracking
    pub timestamp_bytes: [u8; 6],   // Nanoseconds since midnight (48-bit)
    pub order_reference: u64,       // Matches original Add Order reference
    pub shares: u32,                // Quantity executed
    pub match_number: u64,          // Unique match execution ID
}

/// Message Type 'C' - Order Executed With Price
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ItchOrderExecutedWithPrice {
    pub message_type: c_char,       // 'C'
    pub stock_locate: u16,          // Numeric ticker identifier
    pub tracking_number: u16,       // Internal tracking
    pub timestamp_bytes: [u8; 6],   // Nanoseconds since midnight (48-bit)
    pub order_reference: u64,       // Matches original Add Order reference
    pub shares: u32,                // Quantity executed
    pub match_number: u64,          // Unique match execution ID
    pub printable: c_char,          // 'Y' = Publicly visible, 'N' = Hidden
    pub execution_price: u32,       // Non-standard execution price scaled by 10,000
}

// ==========================================
// 3. ORDER MODIFICATION MESSAGES
// ==========================================

/// Message Type 'X' - Order Cancel
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ItchOrderCancel {
    pub message_type: c_char,       // 'X'
    pub stock_locate: u16,          // Numeric ticker identifier
    pub tracking_number: u16,       // Internal tracking
    pub timestamp_bytes: [u8; 6],   // Nanoseconds since midnight (48-bit)
    pub order_reference: u64,       // Active order ID
    pub canceled_shares: u32,       // Number of shares removed
}

/// Message Type 'D' - Order Delete
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ItchOrderDelete {
    pub message_type: c_char,       // 'D'
    pub stock_locate: u16,          // Numeric ticker identifier
    pub tracking_number: u16,       // Internal tracking
    pub timestamp_bytes: [u8; 6],   // Nanoseconds since midnight (48-bit)
    pub order_reference: u64,       // Active order ID to drop completely
}

/// Message Type 'U' - Order Replace
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ItchOrderReplace {
    pub message_type: c_char,       // 'U'
    pub stock_locate: u16,          // Numeric ticker identifier
    pub tracking_number: u16,       // Internal tracking
    pub timestamp_bytes: [u8; 6],   // Nanoseconds since midnight (48-bit)
    pub original_order_reference: u64, // Old order ID being modified
    pub new_order_reference: u64,   // New unique identifier for this placement
    pub shares: u32,                // Updated total quantity
    pub price: u32,                 // Updated price scaled by 10,000
}

/// Unpacks a 48-bit (6-byte) ITCH timestamp into a standard u64 register.
#[inline(always)]
pub fn unpack_itch_timestamp(bytes: &[u8; 6]) -> u64 {
    ((bytes[0] as u64) << 40) |
    ((bytes[1] as u64) << 32) |
    ((bytes[2] as u64) << 24) |
    ((bytes[3] as u64) << 16) |
    ((bytes[4] as u64) << 8)  |
    (bytes[5] as u64)
}
