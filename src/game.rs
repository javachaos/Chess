use std::error::Error;
use std::fmt;

use arrayvec::ArrayVec;

use crate::types::{Color, Move, MoveParseError, Piece, PieceKind, Square};
use crate::zobrist;

pub const STARTING_POSITION_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

const BISHOP_DIRECTIONS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
const ROOK_DIRECTIONS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const KNIGHT_DELTAS: [(i8, i8); 8] = [
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
];
const KING_DELTAS: [(i8, i8); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];
const MAX_MOVES: usize = 256;

pub(crate) type MoveList = ArrayVec<Move, MAX_MOVES>;

#[derive(Clone, Copy, Debug)]
pub(crate) struct BitIter(u64);

impl Iterator for BitIter {
    type Item = Square;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0 == 0 {
            return None;
        }

        let index = self.0.trailing_zeros() as u8;
        self.0 &= self.0 - 1;
        Some(Square::from_index(index))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CastlingRights {
    pub white_king_side: bool,
    pub white_queen_side: bool,
    pub black_king_side: bool,
    pub black_queen_side: bool,
}

impl CastlingRights {
    fn from_fen(value: &str) -> Result<Self, FenError> {
        if value == "-" {
            return Ok(Self::default());
        }

        let mut rights = Self::default();
        for symbol in value.chars() {
            match symbol {
                'K' => rights.white_king_side = true,
                'Q' => rights.white_queen_side = true,
                'k' => rights.black_king_side = true,
                'q' => rights.black_queen_side = true,
                _ => return Err(FenError::new(format!("invalid castling rights: {value}"))),
            }
        }

        Ok(rights)
    }

    fn to_fen(self) -> String {
        let mut result = String::new();
        if self.white_king_side {
            result.push('K');
        }
        if self.white_queen_side {
            result.push('Q');
        }
        if self.black_king_side {
            result.push('k');
        }
        if self.black_queen_side {
            result.push('q');
        }

        if result.is_empty() {
            result.push('-');
        }

        result
    }

    fn clear_for_color(&mut self, color: Color) {
        match color {
            Color::White => {
                self.white_king_side = false;
                self.white_queen_side = false;
            }
            Color::Black => {
                self.black_king_side = false;
                self.black_queen_side = false;
            }
        }
    }

    fn clear_for_rook_square(&mut self, square: Square) {
        match (square.file(), square.rank()) {
            (0, 0) => self.white_queen_side = false,
            (7, 0) => self.white_king_side = false,
            (0, 7) => self.black_queen_side = false,
            (7, 7) => self.black_king_side = false,
            _ => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    bitboards: [[u64; 6]; 2],
    occupancy: [u64; 2],
    mailbox: [Option<Piece>; 64],
}

impl Default for Board {
    fn default() -> Self {
        Self {
            bitboards: [[0; 6]; 2],
            occupancy: [0; 2],
            mailbox: [None; 64],
        }
    }
}

impl Board {
    pub fn piece_at(&self, square: Square) -> Option<Piece> {
        self.mailbox[square.index()]
    }

    pub fn bitboard(&self, color: Color, kind: PieceKind) -> u64 {
        self.bitboards[color.index()][kind.index()]
    }

    pub fn occupancy(&self, color: Option<Color>) -> u64 {
        match color {
            Some(color) => self.occupancy[color.index()],
            None => self.occupancy[Color::White.index()] | self.occupancy[Color::Black.index()],
        }
    }

    fn clear_square(&mut self, square: Square) {
        if let Some(piece) = self.mailbox[square.index()] {
            let mask = !square.bitboard();
            self.bitboards[piece.color.index()][piece.kind.index()] &= mask;
            self.occupancy[piece.color.index()] &= mask;
            self.mailbox[square.index()] = None;
        }
    }

    fn set_piece(&mut self, square: Square, piece: Option<Piece>) {
        self.clear_square(square);
        if let Some(piece) = piece {
            let mask = square.bitboard();
            self.bitboards[piece.color.index()][piece.kind.index()] |= mask;
            self.occupancy[piece.color.index()] |= mask;
            self.mailbox[square.index()] = Some(piece);
        }
    }

    pub(crate) fn squares_for(&self, color: Color, kind: PieceKind) -> BitIter {
        BitIter(self.bitboards[color.index()][kind.index()])
    }
}

impl fmt::Display for Board {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for rank in (0..8).rev() {
            write!(f, "{} ", rank + 1)?;
            for file in 0..8 {
                let square = Square::from_coords(file, rank).expect("board coordinates are valid");
                match self.piece_at(square) {
                    Some(piece) => write!(f, "{} ", piece.fen_char())?,
                    None => write!(f, ". ")?,
                }
            }
            writeln!(f)?;
        }
        writeln!(f, "  a b c d e f g h")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    board: Board,
    side_to_move: Color,
    castling_rights: CastlingRights,
    en_passant_target: Option<Square>,
    halfmove_clock: u32,
    fullmove_number: u32,
    zobrist_hash: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RookUndo {
    rook_from: Square,
    rook_to: Square,
    rook_piece: Piece,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Undo {
    chess_move: Move,
    moved_piece: Piece,
    captured_piece: Option<Piece>,
    capture_square: Square,
    rook_undo: Option<RookUndo>,
    previous_side_to_move: Color,
    previous_castling_rights: CastlingRights,
    previous_en_passant_target: Option<Square>,
    previous_halfmove_clock: u32,
    previous_fullmove_number: u32,
    previous_zobrist_hash: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NullMoveUndo {
    previous_side_to_move: Color,
    previous_en_passant_target: Option<Square>,
    previous_halfmove_clock: u32,
    previous_fullmove_number: u32,
    previous_zobrist_hash: u64,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Self {
        Self::from_fen(STARTING_POSITION_FEN).expect("default starting position FEN must be valid")
    }

    pub fn from_fen(fen: &str) -> Result<Self, FenError> {
        let fields: Vec<&str> = fen.split_whitespace().collect();
        if fields.len() != 6 {
            return Err(FenError::new(format!(
                "expected 6 FEN fields, found {}",
                fields.len()
            )));
        }

        let mut board = Board::default();
        let ranks: Vec<&str> = fields[0].split('/').collect();
        if ranks.len() != 8 {
            return Err(FenError::new("piece placement must contain 8 ranks"));
        }

        for (rank_offset, rank_text) in ranks.iter().enumerate() {
            let board_rank = 7 - rank_offset as u8;
            let mut file = 0_u8;

            for symbol in rank_text.chars() {
                if symbol.is_ascii_digit() {
                    let empty_squares = symbol
                        .to_digit(10)
                        .ok_or_else(|| FenError::new("invalid digit in piece placement"))?
                        as u8;

                    if empty_squares == 0 || file + empty_squares > 8 {
                        return Err(FenError::new(
                            "invalid empty-square count in piece placement",
                        ));
                    }

                    file += empty_squares;
                    continue;
                }

                let kind = PieceKind::from_fen_char(symbol)
                    .ok_or_else(|| FenError::new(format!("invalid piece designator: {symbol}")))?;
                let color = if symbol.is_ascii_uppercase() {
                    Color::White
                } else {
                    Color::Black
                };
                let square = Square::from_coords(file, board_rank)
                    .ok_or_else(|| FenError::new("piece placement overflowed rank width"))?;
                board.set_piece(square, Some(Piece::new(color, kind)));
                file += 1;
            }

            if file != 8 {
                return Err(FenError::new("each FEN rank must describe exactly 8 files"));
            }
        }

        let side_to_move = match fields[1] {
            "w" => Color::White,
            "b" => Color::Black,
            value => return Err(FenError::new(format!("invalid active color: {value}"))),
        };

        let castling_rights = CastlingRights::from_fen(fields[2])?;
        let en_passant_target = if fields[3] == "-" {
            None
        } else {
            let square = fields[3]
                .parse::<Square>()
                .map_err(|_| FenError::new(format!("invalid en passant square: {}", fields[3])))?;
            if square.rank() != 2 && square.rank() != 5 {
                return Err(FenError::new("en passant target must be on rank 3 or 6"));
            }
            Some(square)
        };

        let halfmove_clock = fields[4]
            .parse::<u32>()
            .map_err(|_| FenError::new(format!("invalid halfmove clock: {}", fields[4])))?;
        let fullmove_number = fields[5]
            .parse::<u32>()
            .map_err(|_| FenError::new(format!("invalid fullmove number: {}", fields[5])))?;
        if fullmove_number == 0 {
            return Err(FenError::new("fullmove number must be at least 1"));
        }

        let game = Self {
            board,
            side_to_move,
            castling_rights,
            en_passant_target,
            halfmove_clock,
            fullmove_number,
            zobrist_hash: 0,
        };
        game.validate_kings()?;
        Ok(Self {
            zobrist_hash: game.compute_zobrist_hash(),
            ..game
        })
    }

    pub fn to_fen(&self) -> String {
        let mut placement = Vec::with_capacity(8);
        for rank in (0..8).rev() {
            let mut row = String::new();
            let mut empty_count = 0_u8;
            for file in 0..8 {
                let square = Square::from_coords(file, rank).expect("board coordinates are valid");
                match self.board.piece_at(square) {
                    Some(piece) => {
                        if empty_count > 0 {
                            row.push(char::from_digit(empty_count as u32, 10).expect("digit fits"));
                            empty_count = 0;
                        }
                        row.push(piece.fen_char());
                    }
                    None => empty_count += 1,
                }
            }
            if empty_count > 0 {
                row.push(char::from_digit(empty_count as u32, 10).expect("digit fits"));
            }
            placement.push(row);
        }

        let active_color = match self.side_to_move {
            Color::White => "w",
            Color::Black => "b",
        };
        let en_passant = self
            .en_passant_target
            .map(Square::to_algebraic)
            .unwrap_or_else(|| "-".to_string());

        format!(
            "{} {} {} {} {} {}",
            placement.join("/"),
            active_color,
            self.castling_rights.to_fen(),
            en_passant,
            self.halfmove_clock,
            self.fullmove_number
        )
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    pub fn castling_rights(&self) -> CastlingRights {
        self.castling_rights
    }

    pub fn en_passant_target(&self) -> Option<Square> {
        self.en_passant_target
    }

    pub fn halfmove_clock(&self) -> u32 {
        self.halfmove_clock
    }

    pub fn fullmove_number(&self) -> u32 {
        self.fullmove_number
    }

    pub(crate) fn zobrist_hash(&self) -> u64 {
        self.zobrist_hash
    }

    pub fn make_move(&mut self, chess_move: Move) -> Result<(), MoveError> {
        if self.legal_moves_mut().contains(&chess_move) {
            self.apply_move_unchecked(chess_move);
            Ok(())
        } else {
            Err(MoveError::IllegalMove(chess_move))
        }
    }

    pub fn make_move_str(&mut self, value: &str) -> Result<(), MoveError> {
        let chess_move = value.parse::<Move>().map_err(MoveError::Parse)?;
        self.make_move(chess_move)
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        let mut working = self.clone();
        working.legal_moves_mut().into_iter().collect()
    }

    pub(crate) fn legal_moves_mut(&mut self) -> MoveList {
        let moving_color = self.side_to_move;
        let pseudo_legal_moves = self.generate_pseudo_legal_moves();
        let mut legal_moves = MoveList::new();

        for candidate in pseudo_legal_moves {
            let undo = self.apply_move_unchecked_with_undo(candidate);
            if !self.is_in_check(moving_color) {
                let _ = legal_moves.try_push(candidate);
            }
            self.unapply_move(undo);
        }

        legal_moves
    }

    pub(crate) fn quiescence_moves_mut(&mut self) -> MoveList {
        let moving_color = self.side_to_move;
        if self.is_in_check(moving_color) {
            return self.legal_moves_mut();
        }

        let pseudo_legal_moves = self.generate_pseudo_legal_moves();
        let mut tactical_moves = MoveList::new();

        for candidate in pseudo_legal_moves {
            if !self.is_tactical_move(candidate) {
                continue;
            }

            let undo = self.apply_move_unchecked_with_undo(candidate);
            if !self.is_in_check(moving_color) {
                let _ = tactical_moves.try_push(candidate);
            }
            self.unapply_move(undo);
        }

        tactical_moves
    }

    pub fn is_in_check(&self, color: Color) -> bool {
        self.king_square(color)
            .is_some_and(|king_square| self.is_square_attacked(king_square, color.opposite()))
    }

    pub fn status(&self) -> GameStatus {
        let legal_moves = self.legal_moves();
        if !legal_moves.is_empty() {
            return GameStatus::Ongoing;
        }

        if self.is_in_check(self.side_to_move) {
            GameStatus::Checkmate {
                winner: self.side_to_move.opposite(),
            }
        } else {
            GameStatus::Stalemate
        }
    }

    pub fn perft(&self, depth: u32) -> u64 {
        let mut working = self.clone();
        working.perft_mut(depth)
    }

    pub(crate) fn perft_mut(&mut self, depth: u32) -> u64 {
        if depth == 0 {
            return 1;
        }

        let legal_moves = self.legal_moves_mut();
        let mut nodes = 0;

        for candidate in legal_moves {
            let undo = self.apply_move_unchecked_with_undo(candidate);
            nodes += self.perft_mut(depth - 1);
            self.unapply_move(undo);
        }

        nodes
    }

    fn validate_kings(&self) -> Result<(), FenError> {
        let white_kings = self
            .board
            .bitboard(Color::White, PieceKind::King)
            .count_ones();
        let black_kings = self
            .board
            .bitboard(Color::Black, PieceKind::King)
            .count_ones();

        if white_kings != 1 {
            return Err(FenError::new(format!(
                "expected exactly one white king, found {white_kings}"
            )));
        }
        if black_kings != 1 {
            return Err(FenError::new(format!(
                "expected exactly one black king, found {black_kings}"
            )));
        }

        Ok(())
    }

    fn generate_pseudo_legal_moves(&self) -> MoveList {
        let mut moves = MoveList::new();
        let color = self.side_to_move;

        for square in self.board.squares_for(color, PieceKind::Pawn) {
            self.generate_pawn_moves(square, color, &mut moves);
        }
        for square in self.board.squares_for(color, PieceKind::Knight) {
            self.generate_knight_moves(square, color, &mut moves);
        }
        for square in self.board.squares_for(color, PieceKind::Bishop) {
            self.generate_sliding_moves(square, color, &BISHOP_DIRECTIONS, &mut moves);
        }
        for square in self.board.squares_for(color, PieceKind::Rook) {
            self.generate_sliding_moves(square, color, &ROOK_DIRECTIONS, &mut moves);
        }
        for square in self.board.squares_for(color, PieceKind::Queen) {
            self.generate_sliding_moves(square, color, &BISHOP_DIRECTIONS, &mut moves);
            self.generate_sliding_moves(square, color, &ROOK_DIRECTIONS, &mut moves);
        }
        for square in self.board.squares_for(color, PieceKind::King) {
            self.generate_king_moves(square, color, &mut moves);
        }

        moves
    }

    fn is_tactical_move(&self, chess_move: Move) -> bool {
        chess_move.promotion.is_some()
            || self.board.piece_at(chess_move.to).is_some()
            || self.is_en_passant_move(chess_move)
    }

    fn is_en_passant_move(&self, chess_move: Move) -> bool {
        let moving_piece = self
            .board
            .piece_at(chess_move.from)
            .expect("move source must contain a piece");
        moving_piece.kind == PieceKind::Pawn
            && self.en_passant_target == Some(chess_move.to)
            && chess_move.from.file() != chess_move.to.file()
            && self.board.piece_at(chess_move.to).is_none()
    }

    fn generate_pawn_moves(&self, from: Square, color: Color, moves: &mut MoveList) {
        let forward = if color == Color::White { 1 } else { -1 };
        let start_rank = if color == Color::White { 1 } else { 6 };
        let promotion_rank = if color == Color::White { 7 } else { 0 };

        if let Some(one_step) = from.offset(0, forward) {
            if self.board.piece_at(one_step).is_none() {
                if one_step.rank() == promotion_rank {
                    self.push_promotions(from, one_step, moves);
                } else {
                    let _ = moves.try_push(Move::new(from, one_step, None));
                }

                if from.rank() == start_rank {
                    if let Some(two_step) = from.offset(0, forward * 2) {
                        if self.board.piece_at(two_step).is_none() {
                            let _ = moves.try_push(Move::new(from, two_step, None));
                        }
                    }
                }
            }
        }

        for file_delta in [-1, 1] {
            if let Some(target) = from.offset(file_delta, forward) {
                if let Some(piece) = self.board.piece_at(target) {
                    if piece.color != color {
                        if target.rank() == promotion_rank {
                            self.push_promotions(from, target, moves);
                        } else {
                            let _ = moves.try_push(Move::new(from, target, None));
                        }
                    }
                } else if self.en_passant_target == Some(target) {
                    let _ = moves.try_push(Move::new(from, target, None));
                }
            }
        }
    }

    fn push_promotions(&self, from: Square, to: Square, moves: &mut MoveList) {
        for promotion in [
            PieceKind::Queen,
            PieceKind::Rook,
            PieceKind::Bishop,
            PieceKind::Knight,
        ] {
            let _ = moves.try_push(Move::new(from, to, Some(promotion)));
        }
    }

    fn generate_knight_moves(&self, from: Square, color: Color, moves: &mut MoveList) {
        let attacks = knight_attack_mask(from);
        let own_occupancy = self.board.occupancy(Some(color));
        for target in BitIter(attacks & !own_occupancy) {
            let _ = moves.try_push(Move::new(from, target, None));
        }
    }

    fn generate_sliding_moves(
        &self,
        from: Square,
        color: Color,
        directions: &[(i8, i8)],
        moves: &mut MoveList,
    ) {
        for &(file_delta, rank_delta) in directions {
            let mut current = from;
            while let Some(next) = current.offset(file_delta, rank_delta) {
                match self.board.piece_at(next) {
                    Some(piece) if piece.color == color => break,
                    Some(_) => {
                        let _ = moves.try_push(Move::new(from, next, None));
                        break;
                    }
                    None => {
                        let _ = moves.try_push(Move::new(from, next, None));
                        current = next;
                    }
                }
            }
        }
    }

    fn generate_king_moves(&self, from: Square, color: Color, moves: &mut MoveList) {
        let attacks = king_attack_mask(from);
        let own_occupancy = self.board.occupancy(Some(color));
        for target in BitIter(attacks & !own_occupancy) {
            let _ = moves.try_push(Move::new(from, target, None));
        }

        if self.is_in_check(color) {
            return;
        }

        self.generate_castling_moves(from, color, moves);
    }

    fn generate_castling_moves(&self, from: Square, color: Color, moves: &mut MoveList) {
        let home_rank = if color == Color::White { 0 } else { 7 };
        let home_square = Square::from_coords(4, home_rank).expect("home king square is valid");
        if from != home_square {
            return;
        }

        let kingside_allowed = match color {
            Color::White => self.castling_rights.white_king_side,
            Color::Black => self.castling_rights.black_king_side,
        };
        if kingside_allowed {
            let f_square =
                Square::from_coords(5, home_rank).expect("valid kingside transit square");
            let g_square = Square::from_coords(6, home_rank).expect("valid kingside target square");
            let rook_square = Square::from_coords(7, home_rank).expect("valid rook square");

            if self.board.piece_at(f_square).is_none()
                && self.board.piece_at(g_square).is_none()
                && self.board.piece_at(rook_square) == Some(Piece::new(color, PieceKind::Rook))
                && !self.is_square_attacked(f_square, color.opposite())
                && !self.is_square_attacked(g_square, color.opposite())
            {
                let _ = moves.try_push(Move::new(from, g_square, None));
            }
        }

        let queenside_allowed = match color {
            Color::White => self.castling_rights.white_queen_side,
            Color::Black => self.castling_rights.black_queen_side,
        };
        if queenside_allowed {
            let b_square = Square::from_coords(1, home_rank).expect("valid queenside clear square");
            let c_square =
                Square::from_coords(2, home_rank).expect("valid queenside target square");
            let d_square =
                Square::from_coords(3, home_rank).expect("valid queenside transit square");
            let rook_square = Square::from_coords(0, home_rank).expect("valid rook square");

            if self.board.piece_at(b_square).is_none()
                && self.board.piece_at(c_square).is_none()
                && self.board.piece_at(d_square).is_none()
                && self.board.piece_at(rook_square) == Some(Piece::new(color, PieceKind::Rook))
                && !self.is_square_attacked(c_square, color.opposite())
                && !self.is_square_attacked(d_square, color.opposite())
            {
                let _ = moves.try_push(Move::new(from, c_square, None));
            }
        }
    }

    fn king_square(&self, color: Color) -> Option<Square> {
        self.board.squares_for(color, PieceKind::King).next()
    }

    fn is_square_attacked(&self, square: Square, attacker: Color) -> bool {
        let pawn_rank_delta = if attacker == Color::White { -1 } else { 1 };
        for file_delta in [-1, 1] {
            if let Some(source) = square.offset(file_delta, pawn_rank_delta) {
                if self.board.piece_at(source) == Some(Piece::new(attacker, PieceKind::Pawn)) {
                    return true;
                }
            }
        }

        for source in BitIter(knight_attack_mask(square)) {
            if self.board.piece_at(source) == Some(Piece::new(attacker, PieceKind::Knight)) {
                return true;
            }
        }

        for &(file_delta, rank_delta) in &BISHOP_DIRECTIONS {
            if self.ray_attacks(
                square,
                attacker,
                file_delta,
                rank_delta,
                &[PieceKind::Bishop, PieceKind::Queen],
            ) {
                return true;
            }
        }

        for &(file_delta, rank_delta) in &ROOK_DIRECTIONS {
            if self.ray_attacks(
                square,
                attacker,
                file_delta,
                rank_delta,
                &[PieceKind::Rook, PieceKind::Queen],
            ) {
                return true;
            }
        }

        for source in BitIter(king_attack_mask(square)) {
            if self.board.piece_at(source) == Some(Piece::new(attacker, PieceKind::King)) {
                return true;
            }
        }

        false
    }

    fn ray_attacks(
        &self,
        square: Square,
        attacker: Color,
        file_delta: i8,
        rank_delta: i8,
        valid_kinds: &[PieceKind],
    ) -> bool {
        let mut current = square;
        while let Some(next) = current.offset(file_delta, rank_delta) {
            match self.board.piece_at(next) {
                Some(piece) if piece.color == attacker && valid_kinds.contains(&piece.kind) => {
                    return true;
                }
                Some(_) => return false,
                None => current = next,
            }
        }
        false
    }

    pub(crate) fn apply_move_unchecked(&mut self, chess_move: Move) {
        let _ = self.apply_move_unchecked_with_undo(chess_move);
    }

    pub(crate) fn apply_move_unchecked_with_undo(&mut self, chess_move: Move) -> Undo {
        let moving_color = self.side_to_move;
        let moving_piece = self
            .board
            .piece_at(chess_move.from)
            .expect("move source must contain a piece");
        let mut zobrist_hash = self.zobrist_hash;
        let previous_side_to_move = self.side_to_move;
        let previous_castling_rights = self.castling_rights;
        let previous_en_passant_target = self.en_passant_target;
        let previous_halfmove_clock = self.halfmove_clock;
        let previous_fullmove_number = self.fullmove_number;
        let previous_zobrist_hash = self.zobrist_hash;

        Self::xor_castling_rights_hash(&mut zobrist_hash, self.castling_rights);
        Self::xor_en_passant_hash(&mut zobrist_hash, self.en_passant_target);
        Self::xor_piece_hash(&mut zobrist_hash, moving_piece, chess_move.from);

        let is_castling = moving_piece.kind == PieceKind::King
            && chess_move.from.file().abs_diff(chess_move.to.file()) == 2;
        let is_en_passant = moving_piece.kind == PieceKind::Pawn
            && self.en_passant_target == Some(chess_move.to)
            && chess_move.from.file() != chess_move.to.file()
            && self.board.piece_at(chess_move.to).is_none();

        let mut captured_piece = self.board.piece_at(chess_move.to);
        let mut capture_square = chess_move.to;
        if is_en_passant {
            capture_square = Square::from_coords(chess_move.to.file(), chess_move.from.rank())
                .expect("en passant capture square must be valid");
            captured_piece = self.board.piece_at(capture_square);
            self.board.set_piece(capture_square, None);
        }
        if let Some(captured_piece) = captured_piece {
            Self::xor_piece_hash(&mut zobrist_hash, captured_piece, capture_square);
        }

        self.board.set_piece(chess_move.from, None);

        let rook_undo = if is_castling {
            let home_rank = chess_move.from.rank();
            let (rook_from_file, rook_to_file) = if chess_move.to.file() == 6 {
                (7, 5)
            } else {
                (0, 3)
            };
            let rook_from = Square::from_coords(rook_from_file, home_rank)
                .expect("castling rook square is valid");
            let rook_to = Square::from_coords(rook_to_file, home_rank)
                .expect("castling rook target is valid");
            let rook_piece = self
                .board
                .piece_at(rook_from)
                .expect("castling rook must exist on rook square");
            Self::xor_piece_hash(&mut zobrist_hash, rook_piece, rook_from);
            self.board.set_piece(rook_from, None);
            self.board.set_piece(rook_to, Some(rook_piece));
            Self::xor_piece_hash(&mut zobrist_hash, rook_piece, rook_to);
            Some(RookUndo {
                rook_from,
                rook_to,
                rook_piece,
            })
        } else {
            None
        };

        let promoted_piece = if moving_piece.kind == PieceKind::Pawn
            && (chess_move.to.rank() == 0 || chess_move.to.rank() == 7)
        {
            Piece::new(
                moving_color,
                chess_move.promotion.unwrap_or(PieceKind::Queen),
            )
        } else {
            moving_piece
        };
        self.board.set_piece(chess_move.to, Some(promoted_piece));
        Self::xor_piece_hash(&mut zobrist_hash, promoted_piece, chess_move.to);

        match moving_piece.kind {
            PieceKind::King => self.castling_rights.clear_for_color(moving_color),
            PieceKind::Rook => self.castling_rights.clear_for_rook_square(chess_move.from),
            PieceKind::Pawn | PieceKind::Knight | PieceKind::Bishop | PieceKind::Queen => {}
        }

        if captured_piece == Some(Piece::new(Color::White, PieceKind::Rook))
            || captured_piece == Some(Piece::new(Color::Black, PieceKind::Rook))
        {
            self.castling_rights.clear_for_rook_square(capture_square);
        }

        self.en_passant_target = if moving_piece.kind == PieceKind::Pawn
            && chess_move.from.rank().abs_diff(chess_move.to.rank()) == 2
        {
            Square::from_coords(
                chess_move.from.file(),
                (chess_move.from.rank() + chess_move.to.rank()) / 2,
            )
        } else {
            None
        };

        self.halfmove_clock = if moving_piece.kind == PieceKind::Pawn || captured_piece.is_some() {
            0
        } else {
            self.halfmove_clock + 1
        };
        if moving_color == Color::Black {
            self.fullmove_number += 1;
        }
        self.side_to_move = moving_color.opposite();
        Self::xor_castling_rights_hash(&mut zobrist_hash, self.castling_rights);
        Self::xor_en_passant_hash(&mut zobrist_hash, self.en_passant_target);
        zobrist_hash ^= zobrist::side_to_move_key();
        self.zobrist_hash = zobrist_hash;

        Undo {
            chess_move,
            moved_piece: moving_piece,
            captured_piece,
            capture_square,
            rook_undo,
            previous_side_to_move,
            previous_castling_rights,
            previous_en_passant_target,
            previous_halfmove_clock,
            previous_fullmove_number,
            previous_zobrist_hash,
        }
    }

    pub(crate) fn unapply_move(&mut self, undo: Undo) {
        self.side_to_move = undo.previous_side_to_move;
        self.castling_rights = undo.previous_castling_rights;
        self.en_passant_target = undo.previous_en_passant_target;
        self.halfmove_clock = undo.previous_halfmove_clock;
        self.fullmove_number = undo.previous_fullmove_number;
        self.zobrist_hash = undo.previous_zobrist_hash;

        self.board.set_piece(undo.chess_move.to, None);

        if let Some(rook_undo) = undo.rook_undo {
            self.board.set_piece(rook_undo.rook_to, None);
            self.board
                .set_piece(rook_undo.rook_from, Some(rook_undo.rook_piece));
        }

        self.board
            .set_piece(undo.chess_move.from, Some(undo.moved_piece));
        if let Some(captured_piece) = undo.captured_piece {
            self.board
                .set_piece(undo.capture_square, Some(captured_piece));
        }
    }

    pub(crate) fn apply_null_move_with_undo(&mut self) -> NullMoveUndo {
        let previous_side_to_move = self.side_to_move;
        let previous_en_passant_target = self.en_passant_target;
        let previous_halfmove_clock = self.halfmove_clock;
        let previous_fullmove_number = self.fullmove_number;
        let previous_zobrist_hash = self.zobrist_hash;
        let mut zobrist_hash = self.zobrist_hash;

        Self::xor_en_passant_hash(&mut zobrist_hash, self.en_passant_target);
        self.en_passant_target = None;
        self.halfmove_clock += 1;
        if self.side_to_move == Color::Black {
            self.fullmove_number += 1;
        }
        self.side_to_move = self.side_to_move.opposite();
        zobrist_hash ^= zobrist::side_to_move_key();
        self.zobrist_hash = zobrist_hash;

        NullMoveUndo {
            previous_side_to_move,
            previous_en_passant_target,
            previous_halfmove_clock,
            previous_fullmove_number,
            previous_zobrist_hash,
        }
    }

    pub(crate) fn unapply_null_move(&mut self, undo: NullMoveUndo) {
        self.side_to_move = undo.previous_side_to_move;
        self.en_passant_target = undo.previous_en_passant_target;
        self.halfmove_clock = undo.previous_halfmove_clock;
        self.fullmove_number = undo.previous_fullmove_number;
        self.zobrist_hash = undo.previous_zobrist_hash;
    }

    fn compute_zobrist_hash(&self) -> u64 {
        let mut hash = 0_u64;

        for color in [Color::White, Color::Black] {
            for kind in PieceKind::ALL {
                for square in self.board.squares_for(color, kind) {
                    hash ^= zobrist::piece_key(color, kind, square);
                }
            }
        }

        if self.side_to_move == Color::Black {
            hash ^= zobrist::side_to_move_key();
        }
        if self.castling_rights.white_king_side {
            hash ^= zobrist::castling_key(Color::White, true);
        }
        if self.castling_rights.white_queen_side {
            hash ^= zobrist::castling_key(Color::White, false);
        }
        if self.castling_rights.black_king_side {
            hash ^= zobrist::castling_key(Color::Black, true);
        }
        if self.castling_rights.black_queen_side {
            hash ^= zobrist::castling_key(Color::Black, false);
        }
        if let Some(en_passant_square) = self.en_passant_target {
            hash ^= zobrist::en_passant_file_key(en_passant_square.file());
        }

        hash
    }

    fn xor_piece_hash(hash: &mut u64, piece: Piece, square: Square) {
        *hash ^= zobrist::piece_key(piece.color, piece.kind, square);
    }

    fn xor_castling_rights_hash(hash: &mut u64, rights: CastlingRights) {
        if rights.white_king_side {
            *hash ^= zobrist::castling_key(Color::White, true);
        }
        if rights.white_queen_side {
            *hash ^= zobrist::castling_key(Color::White, false);
        }
        if rights.black_king_side {
            *hash ^= zobrist::castling_key(Color::Black, true);
        }
        if rights.black_queen_side {
            *hash ^= zobrist::castling_key(Color::Black, false);
        }
    }

    fn xor_en_passant_hash(hash: &mut u64, en_passant_target: Option<Square>) {
        if let Some(en_passant_square) = en_passant_target {
            *hash ^= zobrist::en_passant_file_key(en_passant_square.file());
        }
    }
}

fn knight_attack_mask(square: Square) -> u64 {
    let mut mask = 0_u64;
    for (file_delta, rank_delta) in KNIGHT_DELTAS {
        if let Some(target) = square.offset(file_delta, rank_delta) {
            mask |= target.bitboard();
        }
    }
    mask
}

fn king_attack_mask(square: Square) -> u64 {
    let mut mask = 0_u64;
    for (file_delta, rank_delta) in KING_DELTAS {
        if let Some(target) = square.offset(file_delta, rank_delta) {
            mask |= target.bitboard();
        }
    }
    mask
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameStatus {
    Ongoing,
    Checkmate { winner: Color },
    Stalemate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveError {
    Parse(MoveParseError),
    IllegalMove(Move),
}

impl fmt::Display for MoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(source) => write!(f, "{source}"),
            Self::IllegalMove(chess_move) => write!(f, "illegal move: {chess_move}"),
        }
    }
}

impl Error for MoveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(source) => Some(source),
            Self::IllegalMove(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FenError {
    message: String,
}

impl FenError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for FenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for FenError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_position_round_trips_to_fen() {
        let game = Game::new();
        assert_eq!(game.to_fen(), STARTING_POSITION_FEN);
    }

    #[test]
    fn starting_board_tracks_piece_sets_with_bitboards() {
        let game = Game::new();
        assert_eq!(
            game.board()
                .bitboard(Color::White, PieceKind::Pawn)
                .count_ones(),
            8
        );
        assert_eq!(
            game.board()
                .bitboard(Color::Black, PieceKind::Knight)
                .count_ones(),
            2
        );
        assert_eq!(game.board().occupancy(None).count_ones(), 32);
    }

    #[test]
    fn board_mailbox_and_bitboards_stay_in_sync() {
        let mut game = Game::new();

        for value in ["e2e4", "d7d5", "e4d5", "d8d5", "b1c3"] {
            game.make_move_str(value).expect("move should be legal");
        }

        for rank in 0..8 {
            for file in 0..8 {
                let square = Square::from_coords(file, rank).expect("board coordinates are valid");
                let piece = game.board().piece_at(square);
                let mask = square.bitboard();

                match piece {
                    Some(piece) => {
                        assert_ne!(
                            game.board().bitboard(piece.color, piece.kind) & mask,
                            0,
                            "bitboard should contain {piece:?} on {square}"
                        );
                        assert_ne!(
                            game.board().occupancy(Some(piece.color)) & mask,
                            0,
                            "occupancy should contain {piece:?} on {square}"
                        );
                    }
                    None => {
                        assert_eq!(
                            game.board().occupancy(None) & mask,
                            0,
                            "empty square {square} should not appear in occupancy"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn apply_and_unapply_move_restores_position() {
        let mut game = Game::new();
        let original_fen = game.to_fen();
        let original_hash = game.zobrist_hash;

        let undo = game.apply_move_unchecked_with_undo("e2e4".parse().expect("valid move"));
        assert_eq!(game.zobrist_hash, game.compute_zobrist_hash());
        game.unapply_move(undo);

        assert_eq!(game.to_fen(), original_fen);
        assert_eq!(game.zobrist_hash, original_hash);
    }

    #[test]
    fn apply_and_unapply_null_move_restores_position() {
        let mut game = Game::new();
        game.make_move_str("e2e4").expect("move should be legal");
        let original_fen = game.to_fen();
        let original_hash = game.zobrist_hash;

        let undo = game.apply_null_move_with_undo();
        assert_eq!(game.zobrist_hash, game.compute_zobrist_hash());
        game.unapply_null_move(undo);

        assert_eq!(game.to_fen(), original_fen);
        assert_eq!(game.zobrist_hash, original_hash);
    }

    #[test]
    fn incremental_zobrist_hash_matches_recomputed_hash() {
        let mut game = Game::new();

        for value in ["e2e4", "c7c5", "g1f3", "d7d6", "f1b5"] {
            game.make_move_str(value).expect("move should be legal");
            assert_eq!(game.zobrist_hash, game.compute_zobrist_hash());
        }
    }

    #[test]
    fn starting_position_has_twenty_legal_moves() {
        let game = Game::new();
        assert_eq!(game.legal_moves().len(), 20);
    }

    #[test]
    fn starting_position_perft_depth_two_matches_reference() {
        let game = Game::new();
        assert_eq!(game.perft(2), 400);
    }

    #[test]
    fn starting_position_perft_depth_three_matches_reference() {
        let game = Game::new();
        assert_eq!(game.perft(3), 8_902);
    }

    #[test]
    fn en_passant_capture_is_legal_when_available() {
        let game = Game::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1").expect("valid test FEN");
        let legal_moves = game.legal_moves();

        assert!(legal_moves.contains(&Move::new(
            "e5".parse().expect("valid square"),
            "d6".parse().expect("valid square"),
            None,
        )));
    }

    #[test]
    fn both_castles_are_available_in_clear_rook_position() {
        let game = Game::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").expect("valid test FEN");
        let legal_moves = game.legal_moves();

        assert!(legal_moves.contains(&Move::new(
            "e1".parse().expect("valid square"),
            "g1".parse().expect("valid square"),
            None,
        )));
        assert!(legal_moves.contains(&Move::new(
            "e1".parse().expect("valid square"),
            "c1".parse().expect("valid square"),
            None,
        )));
    }

    #[test]
    fn fools_mate_is_checkmate_for_black() {
        let mut game = Game::new();
        for value in ["f2f3", "e7e5", "g2g4", "d8h4"] {
            game.make_move_str(value).expect("move should be legal");
        }

        assert_eq!(
            game.status(),
            GameStatus::Checkmate {
                winner: Color::Black
            }
        );
    }
}
