#![recursion_limit = "256"]

pub mod app_icon;
mod game;
mod search;
mod types;
pub mod ui_app;
mod zobrist;

#[cfg(target_arch = "wasm32")]
mod web;

pub use crate::game::{
    Board, CastlingRights, FenError, Game, GameStatus, MoveError, STARTING_POSITION_FEN,
};
pub use crate::search::{SearchConfig, SearchResult, Searcher};
pub use crate::types::{Color, Move, MoveParseError, Piece, PieceKind, Square, SquareParseError};
