use crate::types::{Color, PieceKind, Square};

const ZOBRIST_SEED: u64 = 0x9e37_79b9_7f4a_7c15;

pub(crate) const fn piece_key(color: Color, kind: PieceKind, square: Square) -> u64 {
    let color_code = match color {
        Color::White => 0_u64,
        Color::Black => 1_u64,
    };
    let kind_code = kind.index() as u64;
    mix(1 + color_code * 512 + kind_code * 64 + square.index() as u64)
}

pub(crate) const fn side_to_move_key() -> u64 {
    mix(10_001)
}

pub(crate) const fn castling_key(color: Color, king_side: bool) -> u64 {
    let color_code = match color {
        Color::White => 0_u64,
        Color::Black => 1_u64,
    };
    let side_code = if king_side { 0_u64 } else { 1_u64 };
    mix(20_001 + color_code * 2 + side_code)
}

pub(crate) const fn en_passant_file_key(file: u8) -> u64 {
    mix(30_001 + file as u64)
}

const fn mix(value: u64) -> u64 {
    splitmix64(ZOBRIST_SEED ^ value.wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

const fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
