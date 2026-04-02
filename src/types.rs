use std::error::Error;
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub const fn opposite(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::White => 0,
            Self::Black => 1,
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::White => f.write_str("White"),
            Self::Black => f.write_str("Black"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PieceKind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl PieceKind {
    pub(crate) const ALL: [Self; 6] = [
        Self::Pawn,
        Self::Knight,
        Self::Bishop,
        Self::Rook,
        Self::Queen,
        Self::King,
    ];

    pub(crate) fn from_fen_char(value: char) -> Option<Self> {
        match value.to_ascii_lowercase() {
            'p' => Some(Self::Pawn),
            'n' => Some(Self::Knight),
            'b' => Some(Self::Bishop),
            'r' => Some(Self::Rook),
            'q' => Some(Self::Queen),
            'k' => Some(Self::King),
            _ => None,
        }
    }

    pub(crate) fn to_fen_char(self, color: Color) -> char {
        let symbol = match self {
            Self::Pawn => 'p',
            Self::Knight => 'n',
            Self::Bishop => 'b',
            Self::Rook => 'r',
            Self::Queen => 'q',
            Self::King => 'k',
        };

        match color {
            Color::White => symbol.to_ascii_uppercase(),
            Color::Black => symbol,
        }
    }

    pub(crate) fn from_promotion_char(value: char) -> Option<Self> {
        match value.to_ascii_lowercase() {
            'n' => Some(Self::Knight),
            'b' => Some(Self::Bishop),
            'r' => Some(Self::Rook),
            'q' => Some(Self::Queen),
            _ => None,
        }
    }

    pub(crate) fn promotion_char(self) -> Option<char> {
        match self {
            Self::Knight => Some('n'),
            Self::Bishop => Some('b'),
            Self::Rook => Some('r'),
            Self::Queen => Some('q'),
            Self::Pawn | Self::King => None,
        }
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Pawn => 0,
            Self::Knight => 1,
            Self::Bishop => 2,
            Self::Rook => 3,
            Self::Queen => 4,
            Self::King => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Piece {
    pub color: Color,
    pub kind: PieceKind,
}

impl Piece {
    pub const fn new(color: Color, kind: PieceKind) -> Self {
        Self { color, kind }
    }

    pub(crate) fn fen_char(self) -> char {
        self.kind.to_fen_char(self.color)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Square(u8);

impl Square {
    pub(crate) const fn from_index(index: u8) -> Self {
        Self(index)
    }

    pub fn from_coords(file: u8, rank: u8) -> Option<Self> {
        if file < 8 && rank < 8 {
            Some(Self(rank * 8 + file))
        } else {
            None
        }
    }

    pub fn file(self) -> u8 {
        self.0 % 8
    }

    pub fn rank(self) -> u8 {
        self.0 / 8
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) const fn bitboard(self) -> u64 {
        1_u64 << self.0
    }

    pub fn offset(self, file_delta: i8, rank_delta: i8) -> Option<Self> {
        let file = self.file() as i8 + file_delta;
        let rank = self.rank() as i8 + rank_delta;

        if (0..=7).contains(&file) && (0..=7).contains(&rank) {
            Self::from_coords(file as u8, rank as u8)
        } else {
            None
        }
    }

    pub fn to_algebraic(self) -> String {
        let file = (b'a' + self.file()) as char;
        let rank = (b'1' + self.rank()) as char;
        format!("{file}{rank}")
    }
}

impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_algebraic())
    }
}

impl FromStr for Square {
    type Err = SquareParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let bytes = value.as_bytes();
        if bytes.len() != 2 {
            return Err(SquareParseError::new(value));
        }

        let file = bytes[0];
        let rank = bytes[1];

        if !(b'a'..=b'h').contains(&file) || !(b'1'..=b'8').contains(&rank) {
            return Err(SquareParseError::new(value));
        }

        let file_index = file - b'a';
        let rank_index = rank - b'1';
        Ok(Self::from_coords(file_index, rank_index).expect("validated square coordinates"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SquareParseError {
    value: String,
}

impl SquareParseError {
    fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

impl fmt::Display for SquareParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid square: {}", self.value)
    }
}

impl Error for SquareParseError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    pub from: Square,
    pub to: Square,
    pub promotion: Option<PieceKind>,
}

impl Move {
    pub const fn new(from: Square, to: Square, promotion: Option<PieceKind>) -> Self {
        Self {
            from,
            to,
            promotion,
        }
    }

    pub fn to_uci(self) -> String {
        let mut result = format!("{}{}", self.from, self.to);
        if let Some(promotion) = self.promotion.and_then(PieceKind::promotion_char) {
            result.push(promotion);
        }
        result
    }
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_uci())
    }
}

impl FromStr for Move {
    type Err = MoveParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 4 && value.len() != 5 {
            return Err(MoveParseError::new(value));
        }

        let from = value[0..2]
            .parse::<Square>()
            .map_err(|_| MoveParseError::new(value))?;
        let to = value[2..4]
            .parse::<Square>()
            .map_err(|_| MoveParseError::new(value))?;
        let promotion = if value.len() == 5 {
            let promotion_char = value.chars().nth(4).expect("validated move length");
            Some(
                PieceKind::from_promotion_char(promotion_char)
                    .ok_or_else(|| MoveParseError::new(value))?,
            )
        } else {
            None
        };

        Ok(Self::new(from, to, promotion))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveParseError {
    value: String,
}

impl MoveParseError {
    fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

impl fmt::Display for MoveParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid UCI move: {}", self.value)
    }
}

impl Error for MoveParseError {}
