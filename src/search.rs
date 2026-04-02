use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

#[cfg(not(target_arch = "wasm32"))]
use std::thread;

use crate::game::{Game, MoveList};
use crate::types::{Color, Move, PieceKind, Square};
use web_time::{Duration, Instant};

const INF: i32 = 1_000_000;
const MATE_SCORE: i32 = 30_000;
const PIECE_VALUES: [i32; 6] = [100, 320, 330, 500, 900, 0];
const PARALLEL_SEARCH_MIN_DEPTH: u8 = 3;
const DEFAULT_TT_CAPACITY: usize = 1 << 18;
const HISTORY_TABLE_SIZE: usize = 64 * 64;
const MAX_HISTORY_SCORE: i32 = 1_000_000;
const NULL_MOVE_MIN_DEPTH: u8 = 3;
const NULL_MOVE_BASE_REDUCTION: u8 = 2;
const LMR_MIN_DEPTH: u8 = 3;
const LMR_MIN_MOVE_INDEX: usize = 3;
const MAX_QUIESCENCE_PLY: i32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchConfig {
    pub depth: u8,
    pub parallelism: Option<usize>,
    pub transposition_table_capacity: usize,
    pub time_limit: Option<Duration>,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            depth: 4,
            parallelism: None,
            transposition_table_capacity: DEFAULT_TT_CAPACITY,
            time_limit: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub best_move: Option<Move>,
    pub score: i32,
    pub nodes: u64,
    pub depth: u8,
}

pub struct Searcher {
    nodes: u64,
    transposition_table: TranspositionTable,
    killer_moves: Vec<[Option<Move>; 2]>,
    history_scores: [[i32; HISTORY_TABLE_SIZE]; 2],
    deadline: Option<Instant>,
    stop_flag: Arc<AtomicBool>,
}

impl Default for Searcher {
    fn default() -> Self {
        Self {
            nodes: 0,
            transposition_table: TranspositionTable::new(DEFAULT_TT_CAPACITY),
            killer_moves: Vec::new(),
            history_scores: [[0; HISTORY_TABLE_SIZE]; 2],
            deadline: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bound {
    Exact,
    Lower,
    Upper,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TranspositionEntry {
    key: u64,
    depth: u8,
    score: i32,
    bound: Bound,
    best_move: Option<Move>,
}

impl TranspositionEntry {
    const MOVE_BITS: u64 = 16;
    const SCORE_BITS: u64 = 21;
    const BOUND_BITS: u64 = 2;
    const DEPTH_BITS: u64 = 8;
    const FINGERPRINT_BITS: u64 = 16;

    const MOVE_SHIFT: u64 = 0;
    const SCORE_SHIFT: u64 = Self::MOVE_SHIFT + Self::MOVE_BITS;
    const BOUND_SHIFT: u64 = Self::SCORE_SHIFT + Self::SCORE_BITS;
    const DEPTH_SHIFT: u64 = Self::BOUND_SHIFT + Self::BOUND_BITS;
    const FINGERPRINT_SHIFT: u64 = Self::DEPTH_SHIFT + Self::DEPTH_BITS;

    const SCORE_MASK: u64 = (1_u64 << Self::SCORE_BITS) - 1;
    const DEPTH_MASK: u64 = (1_u64 << Self::DEPTH_BITS) - 1;
    const FINGERPRINT_MASK: u64 = (1_u64 << Self::FINGERPRINT_BITS) - 1;

    fn pack(self) -> u64 {
        let fingerprint = Self::fingerprint(self.key) as u64;
        let score = Self::pack_score(self.score);
        let bound = Self::pack_bound(self.bound) as u64;
        let best_move = encode_tt_move(self.best_move) as u64;

        (fingerprint << Self::FINGERPRINT_SHIFT)
            | (u64::from(self.depth) << Self::DEPTH_SHIFT)
            | (bound << Self::BOUND_SHIFT)
            | (score << Self::SCORE_SHIFT)
            | (best_move << Self::MOVE_SHIFT)
    }

    fn unpack(packed: u64, key: u64) -> Option<Self> {
        if packed == 0 || Self::packed_fingerprint(packed) != Self::fingerprint(key) {
            return None;
        }

        Some(Self {
            key,
            depth: Self::packed_depth(packed),
            score: Self::unpack_score(packed),
            bound: Self::unpack_bound(packed),
            best_move: decode_tt_move(packed as u16),
        })
    }

    fn packed_depth(packed: u64) -> u8 {
        ((packed >> Self::DEPTH_SHIFT) & Self::DEPTH_MASK) as u8
    }

    fn packed_fingerprint(packed: u64) -> u16 {
        ((packed >> Self::FINGERPRINT_SHIFT) & Self::FINGERPRINT_MASK) as u16
    }

    fn fingerprint(key: u64) -> u16 {
        let mixed = key ^ (key >> 16) ^ (key >> 32) ^ (key >> 48);
        let fingerprint = (mixed as u16).wrapping_add(1);
        if fingerprint == 0 { 1 } else { fingerprint }
    }

    fn pack_score(score: i32) -> u64 {
        debug_assert!((-1_048_576..=1_048_575).contains(&score));
        ((score as i64) & Self::SCORE_MASK as i64) as u64
    }

    fn unpack_score(packed: u64) -> i32 {
        let raw = ((packed >> Self::SCORE_SHIFT) & Self::SCORE_MASK) as i32;
        let sign_bit = 1_i32 << (Self::SCORE_BITS as i32 - 1);
        if raw & sign_bit != 0 {
            raw - (1_i32 << Self::SCORE_BITS)
        } else {
            raw
        }
    }

    fn pack_bound(bound: Bound) -> u8 {
        match bound {
            Bound::Exact => 0,
            Bound::Lower => 1,
            Bound::Upper => 2,
        }
    }

    fn unpack_bound(packed: u64) -> Bound {
        match ((packed >> Self::BOUND_SHIFT) & ((1_u64 << Self::BOUND_BITS) - 1)) as u8 {
            1 => Bound::Lower,
            2 => Bound::Upper,
            _ => Bound::Exact,
        }
    }
}

fn encode_tt_move(best_move: Option<Move>) -> u16 {
    let Some(best_move) = best_move else {
        return 0;
    };

    let promotion = match best_move.promotion {
        None => 0,
        Some(PieceKind::Knight) => 1,
        Some(PieceKind::Bishop) => 2,
        Some(PieceKind::Rook) => 3,
        Some(PieceKind::Queen) => 4,
        Some(PieceKind::Pawn | PieceKind::King) => 0,
    };

    ((best_move.from.index() as u16) << 10)
        | ((best_move.to.index() as u16) << 4)
        | promotion
}

fn decode_tt_move(encoded: u16) -> Option<Move> {
    if encoded == 0 {
        return None;
    }

    let from = Square::from_index(((encoded >> 10) & 0x3f) as u8);
    let to = Square::from_index(((encoded >> 4) & 0x3f) as u8);
    let promotion = match encoded & 0x0f {
        0 => None,
        1 => Some(PieceKind::Knight),
        2 => Some(PieceKind::Bishop),
        3 => Some(PieceKind::Rook),
        4 => Some(PieceKind::Queen),
        _ => None,
    };

    Some(Move::new(from, to, promotion))
}

#[derive(Clone, Debug)]
struct TranspositionTable {
    entries: Arc<[AtomicU64]>,
}

impl TranspositionTable {
    fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let mut entries = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            entries.push(AtomicU64::new(0));
        }

        Self {
            entries: Arc::from(entries.into_boxed_slice()),
        }
    }

    fn get(&self, key: u64) -> Option<TranspositionEntry> {
        let packed = self.entries[self.index(key)].load(Ordering::Relaxed);
        TranspositionEntry::unpack(packed, key)
    }

    fn store(&self, entry: TranspositionEntry) {
        let index = self.index(entry.key);
        let slot = &self.entries[index];
        let packed = entry.pack();

        loop {
            let current = slot.load(Ordering::Relaxed);
            if !Self::should_replace(current, entry) {
                return;
            }

            match slot.compare_exchange_weak(current, packed, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(_) => continue,
            }
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn index(&self, key: u64) -> usize {
        key as usize % self.len()
    }

    fn should_replace(current: u64, entry: TranspositionEntry) -> bool {
        if current == 0 {
            return true;
        }

        entry.depth >= TranspositionEntry::packed_depth(current)
            || entry.bound == Bound::Exact
    }
}

impl Searcher {
    pub fn search(&mut self, game: &Game, config: SearchConfig) -> SearchResult {
        if self.transposition_table.len() != config.transposition_table_capacity.max(1) {
            self.transposition_table =
                TranspositionTable::new(config.transposition_table_capacity.max(1));
        }
        let depth = config.depth.max(1);
        if let Some(book_move) = opening_book_move(game) {
            return SearchResult {
                best_move: Some(book_move),
                score: 0,
                nodes: 0,
                depth,
            };
        }
        self.prepare_for_search(depth, config.time_limit);

        let parallelism = host_parallelism();
        let parallelism = config
            .parallelism
            .unwrap_or(parallelism)
            .max(1);

        let mut result = self.initial_result(game, depth);
        let mut previous_best_move = None;

        for current_depth in 1..=depth {
            if self.abort_requested() {
                break;
            }
            let Some(mut iteration_result) =
                self.search_at_depth(game, current_depth, parallelism, previous_best_move)
            else {
                break;
            };

            iteration_result.nodes = self.nodes;
            previous_best_move = iteration_result.best_move;
            result = iteration_result;

            if result.best_move.is_none() || result.score.abs() >= MATE_SCORE - 128 {
                break;
            }
        }

        result
    }

    fn search_at_depth(
        &mut self,
        game: &Game,
        depth: u8,
        parallelism: usize,
        root_hint: Option<Move>,
    ) -> Option<SearchResult> {
        let use_parallel = depth >= PARALLEL_SEARCH_MIN_DEPTH && parallelism > 1;

        if use_parallel {
            let iteration_result =
                self.search_parallel(game, depth, parallelism, root_hint, -INF, INF);
            self.nodes += iteration_result.as_ref().map_or(0, |result| result.nodes);
            iteration_result
        } else {
            let mut working = game.clone();
            self.search_sequential(&mut working, depth, root_hint, -INF, INF)
        }
    }

    fn search_sequential(
        &mut self,
        game: &mut Game,
        depth: u8,
        root_hint: Option<Move>,
        alpha: i32,
        beta: i32,
    ) -> Option<SearchResult> {
        let start_nodes = self.nodes;
        let position_key = game.zobrist_hash();
        let legal_moves = game.legal_moves_mut();

        if legal_moves.is_empty() {
            let score = terminal_score(game, 0);
            self.transposition_table.store(TranspositionEntry {
                key: position_key,
                depth,
                score,
                bound: Bound::Exact,
                best_move: None,
            });
            return Some(SearchResult {
                best_move: None,
                score,
                nodes: self.nodes - start_nodes,
                depth,
            });
        }

        let tt_move = self
            .transposition_table
            .get(position_key)
            .and_then(|entry| entry.best_move);
        let legal_moves = self.ordered_moves(game, legal_moves, root_hint.or(tt_move), 0);
        let mut best_move = None;
        let mut best_score = -INF;
        let mut alpha = alpha;
        let mut is_first_move = true;

        for candidate in legal_moves {
            if self.abort_requested() {
                return None;
            }
            let score = if is_first_move {
                self.score_root_move(game, candidate, depth, alpha, beta)
            } else {
                let mut score = self.score_root_move(game, candidate, depth, alpha, alpha + 1);
                if score? > alpha {
                    score = self.score_root_move(game, candidate, depth, alpha, beta);
                }
                score
            };
            let score = score?;
            is_first_move = false;
            if score > best_score {
                best_score = score;
                best_move = Some(candidate);
            }
            alpha = alpha.max(score);
        }

        self.transposition_table.store(TranspositionEntry {
            key: position_key,
            depth,
            score: best_score,
            bound: Bound::Exact,
            best_move,
        });

        Some(SearchResult {
            best_move,
            score: best_score,
            nodes: self.nodes - start_nodes,
            depth,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn search_parallel(
        &mut self,
        game: &Game,
        depth: u8,
        worker_count: usize,
        root_hint: Option<Move>,
        alpha: i32,
        beta: i32,
    ) -> Option<SearchResult> {
        let position_key = game.zobrist_hash();
        let mut working = game.clone();
        let legal_moves = working.legal_moves_mut();

        if legal_moves.is_empty() {
            let score = terminal_score(game, 0);
            self.transposition_table.store(TranspositionEntry {
                key: position_key,
                depth,
                score,
                bound: Bound::Exact,
                best_move: None,
            });
            return Some(SearchResult {
                best_move: None,
                score,
                nodes: 0,
                depth,
            });
        }

        let tt_move = self
            .transposition_table
            .get(position_key)
            .and_then(|entry| entry.best_move);
        let legal_moves = self.ordered_moves(game, legal_moves, root_hint.or(tt_move), 0);
        let worker_count = worker_count.min(legal_moves.len());
        let indexed_moves: Vec<(usize, Move)> = legal_moves.into_iter().enumerate().collect();
        let mut thread_results = Vec::new();

        thread::scope(|scope| {
            let mut handles = Vec::new();

            for worker_id in 0..worker_count {
                let worker_moves: Vec<(usize, Move)> = indexed_moves
                    .iter()
                    .copied()
                    .filter(|(index, _)| index % worker_count == worker_id)
                    .collect();

                if worker_moves.is_empty() {
                    continue;
                }

                let seed = self.fork_for_parallel_worker();
                handles.push(scope.spawn(move || {
                    let mut searcher = seed;
                    let mut working = game.clone();
                    let mut best: Option<RootMoveResult> = None;
                    let mut alpha = alpha;
                    let mut is_first_move = true;
                    let mut completed = true;

                    for (root_index, candidate) in worker_moves {
                        if searcher.abort_requested() {
                            completed = false;
                            break;
                        }
                        let score = if is_first_move {
                            searcher.score_root_move(&mut working, candidate, depth, alpha, beta)
                        } else {
                            let mut score = searcher.score_root_move(
                                &mut working,
                                candidate,
                                depth,
                                alpha,
                                alpha + 1,
                            );
                            if score.is_some_and(|score| score > alpha) {
                                score = searcher.score_root_move(
                                    &mut working,
                                    candidate,
                                    depth,
                                    alpha,
                                    beta,
                                );
                            }
                            score
                        };
                        let Some(score) = score else {
                            completed = false;
                            break;
                        };
                        is_first_move = false;
                        let outcome = RootMoveResult {
                            root_index,
                            chess_move: candidate,
                            score,
                        };

                        if best.is_none_or(|current| outcome.is_better_than(current)) {
                            best = Some(outcome);
                        }
                        alpha = alpha.max(score);
                    }

                    ThreadSearchResult {
                        best,
                        nodes: searcher.nodes,
                        completed,
                    }
                }));
            }

            for handle in handles {
                thread_results.push(
                    handle
                        .join()
                        .expect("parallel search worker should succeed"),
                );
            }
        });

        let mut best = None;
        let mut total_nodes = 0;
        for thread_result in thread_results {
            if !thread_result.completed {
                return None;
            }
            total_nodes += thread_result.nodes;
            if let Some(thread_best) = thread_result.best {
                if best.is_none_or(|current| thread_best.is_better_than(current)) {
                    best = Some(thread_best);
                }
            }
        }

        let best = best.expect("parallel search should evaluate at least one move");
        self.transposition_table.store(TranspositionEntry {
            key: position_key,
            depth,
            score: best.score,
            bound: Bound::Exact,
            best_move: Some(best.chess_move),
        });
        Some(SearchResult {
            best_move: Some(best.chess_move),
            score: best.score,
            nodes: total_nodes,
            depth,
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn search_parallel(
        &mut self,
        game: &Game,
        depth: u8,
        _worker_count: usize,
        root_hint: Option<Move>,
        alpha: i32,
        beta: i32,
    ) -> Option<SearchResult> {
        let mut working = game.clone();
        self.search_sequential(&mut working, depth, root_hint, alpha, beta)
    }

    pub fn evaluate(&self, game: &Game) -> i32 {
        let mut score = 0;

        for color in [Color::White, Color::Black] {
            let sign = if color == Color::White { 1 } else { -1 };
            for kind in PieceKind::ALL {
                for square in game.board().squares_for(color, kind) {
                    score += sign * (piece_value(kind) + positional_bonus(kind, square, color));
                }
            }
        }

        score
    }

    fn negamax(
        &mut self,
        game: &mut Game,
        depth: u8,
        ply: i32,
        mut alpha: i32,
        mut beta: i32,
    ) -> Option<i32> {
        self.nodes += 1;
        if self.should_check_abort() && self.abort_requested() {
            return None;
        }
        let original_alpha = alpha;
        let position_key = game.zobrist_hash();

        if let Some(entry) = self.transposition_table.get(position_key) {
            if entry.depth >= depth {
                match entry.bound {
                    Bound::Exact => return Some(entry.score),
                    Bound::Lower => alpha = alpha.max(entry.score),
                    Bound::Upper => beta = beta.min(entry.score),
                }
                if alpha >= beta {
                    return Some(entry.score);
                }
            }
        }

        if depth == 0 {
            return self.quiescence(game, ply, alpha, beta);
        }

        let in_check = game.is_in_check(game.side_to_move());
        let legal_moves = game.legal_moves_mut();

        if legal_moves.is_empty() {
            let score = terminal_score_from_check(in_check, ply);
            self.transposition_table.store(TranspositionEntry {
                key: position_key,
                depth,
                score,
                bound: Bound::Exact,
                best_move: None,
            });
            return Some(score);
        }

        if self.should_try_null_move(game, depth, beta, in_check) {
            let reduction = self.null_move_reduction(depth);
            let undo = game.apply_null_move_with_undo();
            let null_score = -self.negamax(
                game,
                depth.saturating_sub(1 + reduction),
                ply + 1,
                -beta,
                -beta + 1,
            )?;
            game.unapply_null_move(undo);

            if null_score >= beta {
                self.transposition_table.store(TranspositionEntry {
                    key: position_key,
                    depth,
                    score: beta,
                    bound: Bound::Lower,
                    best_move: None,
                });
                return Some(beta);
            }
        }

        let mut best_score = -INF;
        let tt_move = self
            .transposition_table
            .get(position_key)
            .and_then(|entry| entry.best_move);
        let ordered_moves = self.ordered_moves(game, legal_moves, tt_move, ply as usize);
        let mut best_move = None;
        let mut is_first_move = true;

        for (move_index, candidate) in ordered_moves.into_iter().enumerate() {
            let reduction =
                (!is_first_move).then(|| self.late_move_reduction(game, candidate, depth, move_index, in_check)).unwrap_or(0);
            let undo = game.apply_move_unchecked_with_undo(candidate);
            let score = if is_first_move {
                -self.negamax(game, depth - 1, ply + 1, -beta, -alpha)?
            } else {
                let mut score = if reduction > 0 {
                    -self.negamax(
                        game,
                        depth.saturating_sub(1 + reduction),
                        ply + 1,
                        -alpha - 1,
                        -alpha,
                    )?
                } else {
                    -self.negamax(game, depth - 1, ply + 1, -alpha - 1, -alpha)?
                };
                if score > alpha && score < beta {
                    score = -self.negamax(game, depth - 1, ply + 1, -beta, -alpha)?;
                }
                score
            };
            game.unapply_move(undo);
            is_first_move = false;
            if score > best_score {
                best_score = score;
                best_move = Some(candidate);
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                self.record_beta_cutoff(game, candidate, depth, ply as usize);
                break;
            }
        }

        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if best_score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.transposition_table.store(TranspositionEntry {
            key: position_key,
            depth,
            score: best_score,
            bound,
            best_move,
        });

        Some(best_score)
    }

    fn quiescence(&mut self, game: &mut Game, ply: i32, mut alpha: i32, beta: i32) -> Option<i32> {
        if self.should_check_abort() && self.abort_requested() {
            return None;
        }
        let original_alpha = alpha;
        let position_key = game.zobrist_hash();

        if let Some(entry) = self.transposition_table.get(position_key) {
            if entry.depth == 0 {
                match entry.bound {
                    Bound::Exact => return Some(entry.score),
                    Bound::Lower => alpha = alpha.max(entry.score),
                    Bound::Upper => {}
                }
                if alpha >= beta {
                    return Some(entry.score);
                }
            }
        }

        let in_check = game.is_in_check(game.side_to_move());
        if ply >= MAX_QUIESCENCE_PLY {
            return Some(self.evaluate_for_side_to_move(game));
        }

        let mut best_score = if in_check {
            -INF
        } else {
            let stand_pat = self.evaluate_for_side_to_move(game);
            if stand_pat >= beta {
                self.transposition_table.store(TranspositionEntry {
                    key: position_key,
                    depth: 0,
                    score: stand_pat,
                    bound: Bound::Lower,
                    best_move: None,
                });
                return Some(stand_pat);
            }
            alpha = alpha.max(stand_pat);
            stand_pat
        };

        let moves = game.quiescence_moves_mut();
        if moves.is_empty() {
            let score = if in_check {
                terminal_score_from_check(true, ply)
            } else {
                best_score
            };
            self.transposition_table.store(TranspositionEntry {
                key: position_key,
                depth: 0,
                score,
                bound: Bound::Exact,
                best_move: None,
            });
            return Some(score);
        }

        let tt_move = self
            .transposition_table
            .get(position_key)
            .and_then(|entry| entry.best_move);
        let ordered_moves = self.ordered_moves(game, moves, tt_move, ply as usize);
        let mut best_move = None;

        for candidate in ordered_moves {
            if self.abort_requested() {
                return None;
            }
            let undo = game.apply_move_unchecked_with_undo(candidate);
            self.nodes += 1;
            let score = -self.quiescence(game, ply + 1, -beta, -alpha)?;
            game.unapply_move(undo);

            if score > best_score {
                best_score = score;
                best_move = Some(candidate);
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }

        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if best_score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.transposition_table.store(TranspositionEntry {
            key: position_key,
            depth: 0,
            score: best_score,
            bound,
            best_move,
        });

        Some(best_score)
    }

    fn score_root_move(
        &mut self,
        game: &mut Game,
        candidate: Move,
        depth: u8,
        alpha: i32,
        beta: i32,
    ) -> Option<i32> {
        let undo = game.apply_move_unchecked_with_undo(candidate);
        let score = -self.negamax(game, depth - 1, 1, -beta, -alpha)?;
        game.unapply_move(undo);
        Some(score)
    }

    fn evaluate_for_side_to_move(&self, game: &Game) -> i32 {
        let score = self.evaluate(game);
        if game.side_to_move() == Color::White {
            score
        } else {
            -score
        }
    }

    fn should_try_null_move(&self, game: &Game, depth: u8, beta: i32, in_check: bool) -> bool {
        depth >= NULL_MOVE_MIN_DEPTH
            && !in_check
            && beta.abs() < MATE_SCORE - 256
            && has_non_pawn_material(game, game.side_to_move())
            && self.evaluate_for_side_to_move(game) >= beta
    }

    fn null_move_reduction(&self, depth: u8) -> u8 {
        NULL_MOVE_BASE_REDUCTION + depth / 4
    }

    fn late_move_reduction(
        &self,
        game: &Game,
        candidate: Move,
        depth: u8,
        move_index: usize,
        in_check: bool,
    ) -> u8 {
        if depth < LMR_MIN_DEPTH
            || in_check
            || move_index < LMR_MIN_MOVE_INDEX
            || !self.is_quiet_move(game, candidate)
        {
            return 0;
        }

        let mut reduction = 1;
        if depth >= 6 && move_index >= 6 {
            reduction += 1;
        }

        reduction.min(depth.saturating_sub(2))
    }

    fn ordered_moves(
        &self,
        game: &Game,
        mut moves: MoveList,
        preferred_move: Option<Move>,
        ply: usize,
    ) -> MoveList {
        moves.sort_by_key(|candidate| -self.move_order_score(game, *candidate, preferred_move, ply));
        moves
    }

    fn move_order_score(
        &self,
        game: &Game,
        candidate: Move,
        preferred_move: Option<Move>,
        ply: usize,
    ) -> i32 {
        let board = game.board();
        let moving_piece = board
            .piece_at(candidate.from)
            .expect("move source should contain a piece");
        let mut score = if preferred_move == Some(candidate) {
            20_000_000
        } else {
            0
        };

        if let Some(captured_kind) = self.captured_piece_kind(game, candidate, moving_piece.kind) {
            score += 8_000_000 + 64 * piece_value(captured_kind) - piece_value(moving_piece.kind);
        }

        if let Some(promotion) = candidate.promotion {
            score += 6_000_000 + piece_value(promotion);
        }

        if moving_piece.kind == PieceKind::King
            && candidate.from.file().abs_diff(candidate.to.file()) == 2
        {
            score += 40_000;
        }

        if let Some(killers) = self.killer_moves.get(ply) {
            if killers[0] == Some(candidate) {
                score += 4_000_000;
            } else if killers[1] == Some(candidate) {
                score += 3_500_000;
            }
        }

        score += self.history_score(game.side_to_move(), candidate).min(MAX_HISTORY_SCORE);

        score
    }

    fn prepare_for_search(&mut self, depth: u8, time_limit: Option<Duration>) {
        self.nodes = 0;
        self.deadline = time_limit.map(|limit| Instant::now() + limit);
        self.stop_flag = Arc::new(AtomicBool::new(false));
        let killer_slots = depth as usize + 4;
        if self.killer_moves.len() < killer_slots {
            self.killer_moves.resize(killer_slots, [None, None]);
        }
        for killers in &mut self.killer_moves {
            *killers = [None, None];
        }
        self.history_scores = [[0; HISTORY_TABLE_SIZE]; 2];
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn fork_for_parallel_worker(&self) -> Self {
        Self {
            nodes: 0,
            transposition_table: self.transposition_table.clone(),
            killer_moves: self.killer_moves.clone(),
            history_scores: self.history_scores,
            deadline: self.deadline,
            stop_flag: Arc::clone(&self.stop_flag),
        }
    }

    fn initial_result(&self, game: &Game, depth: u8) -> SearchResult {
        let mut working = game.clone();
        let legal_moves = working.legal_moves_mut();

        if legal_moves.is_empty() {
            return SearchResult {
                best_move: None,
                score: terminal_score(game, 0),
                nodes: 0,
                depth,
            };
        }

        let tt_move = self
            .transposition_table
            .get(working.zobrist_hash())
            .and_then(|entry| entry.best_move);
        let best_move = self
            .ordered_moves(&working, legal_moves, tt_move, 0)
            .into_iter()
            .next();

        SearchResult {
            best_move,
            score: self.evaluate_for_side_to_move(&working),
            nodes: 0,
            depth,
        }
    }

    fn should_check_abort(&self) -> bool {
        self.nodes & 1_023 == 0
    }

    fn abort_requested(&self) -> bool {
        if self.stop_flag.load(Ordering::Relaxed) {
            return true;
        }
        if let Some(deadline) = self.deadline {
            if Instant::now() >= deadline {
                self.stop_flag.store(true, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    fn captured_piece_kind(
        &self,
        game: &Game,
        candidate: Move,
        moving_kind: PieceKind,
    ) -> Option<PieceKind> {
        game.board()
            .piece_at(candidate.to)
            .map(|piece| piece.kind)
            .or_else(|| {
                (moving_kind == PieceKind::Pawn
                    && game.en_passant_target() == Some(candidate.to)
                    && candidate.from.file() != candidate.to.file())
                .then_some(PieceKind::Pawn)
            })
    }

    fn record_beta_cutoff(&mut self, game: &Game, candidate: Move, depth: u8, ply: usize) {
        if !self.is_quiet_move(game, candidate) {
            return;
        }

        self.add_killer_move(ply, candidate);
        let history_slot = &mut self.history_scores[game.side_to_move().index()]
            [candidate.from.index() * 64 + candidate.to.index()];
        let bonus = (depth as i32) * (depth as i32) * 16;
        *history_slot = (*history_slot + bonus).min(MAX_HISTORY_SCORE);
    }

    fn add_killer_move(&mut self, ply: usize, candidate: Move) {
        if ply >= self.killer_moves.len() {
            self.killer_moves.resize(ply + 1, [None, None]);
        }

        let killers = &mut self.killer_moves[ply];
        if killers[0] == Some(candidate) {
            return;
        }
        killers[1] = killers[0];
        killers[0] = Some(candidate);
    }

    fn history_score(&self, color: Color, candidate: Move) -> i32 {
        self.history_scores[color.index()][candidate.from.index() * 64 + candidate.to.index()]
    }

    fn is_quiet_move(&self, game: &Game, candidate: Move) -> bool {
        candidate.promotion.is_none()
            && game.board().piece_at(candidate.to).is_none()
            && !(game.en_passant_target() == Some(candidate.to)
                && candidate.from.file() != candidate.to.file())
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RootMoveResult {
    root_index: usize,
    chess_move: Move,
    score: i32,
}

#[cfg(not(target_arch = "wasm32"))]
impl RootMoveResult {
    fn is_better_than(self, other: Self) -> bool {
        self.score > other.score
            || (self.score == other.score && self.root_index < other.root_index)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ThreadSearchResult {
    best: Option<RootMoveResult>,
    nodes: u64,
    completed: bool,
}

impl Game {
    pub fn best_move(&self, depth: u8) -> SearchResult {
        self.best_move_with_config(SearchConfig {
            depth,
            ..SearchConfig::default()
        })
    }

    pub fn best_move_with_config(&self, config: SearchConfig) -> SearchResult {
        Searcher::default().search(self, config)
    }

    pub fn evaluate(&self) -> i32 {
        Searcher::default().evaluate(self)
    }
}

fn terminal_score(game: &Game, ply: i32) -> i32 {
    if game.is_in_check(game.side_to_move()) {
        -MATE_SCORE + ply
    } else {
        0
    }
}

fn terminal_score_from_check(in_check: bool, ply: i32) -> i32 {
    if in_check {
        -MATE_SCORE + ply
    } else {
        0
    }
}

fn host_parallelism() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        1
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1)
    }
}

fn piece_value(kind: PieceKind) -> i32 {
    PIECE_VALUES[kind.index()]
}

fn has_non_pawn_material(game: &Game, color: Color) -> bool {
    [
        PieceKind::Knight,
        PieceKind::Bishop,
        PieceKind::Rook,
        PieceKind::Queen,
    ]
    .into_iter()
    .any(|kind| game.board().bitboard(color, kind) != 0)
}

fn positional_bonus(kind: PieceKind, square: Square, color: Color) -> i32 {
    let forward_rank = match color {
        Color::White => square.rank() as i32,
        Color::Black => 7 - square.rank() as i32,
    };
    let file_distance = (square.file() as i32 - 3)
        .abs()
        .min((square.file() as i32 - 4).abs());
    let rank_distance = (square.rank() as i32 - 3)
        .abs()
        .min((square.rank() as i32 - 4).abs());
    let center_bonus = 3 - file_distance - rank_distance;

    match kind {
        PieceKind::Pawn => forward_rank * 8 + center_bonus.max(-2) * 2,
        PieceKind::Knight => center_bonus * 12,
        PieceKind::Bishop => center_bonus * 8,
        PieceKind::Rook => forward_rank * 3,
        PieceKind::Queen => center_bonus * 4,
        PieceKind::King => {
            let home_rank_penalty = if forward_rank > 1 { -20 } else { 0 };
            home_rank_penalty - center_bonus * 6
        }
    }
}

fn opening_book_move(game: &Game) -> Option<Move> {
    let book_move = match opening_book_key(game).as_str() {
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -" => "e2e4",
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -" => "c7c5",
        "rnbqkbnr/pppppppp/8/8/3PP3/8/PPP2PPP/RNBQKBNR b KQkq -" => "d7d5",
        "rnbqkbnr/pppppppp/8/8/2P5/8/PP1PPPPP/RNBQKBNR b KQkq -" => "e7e5",
        "rnbqkbnr/pppppppp/8/8/8/5N2/PPPPPPPP/RNBQKB1R b KQkq -" => "d7d5",
        "rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq -" => "g1f3",
        "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq -" => "g1f3",
        "rnbqkbnr/pp1ppppp/8/2p5/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq -" => "d7d6",
        "rnbqkbnr/pp1ppppp/3p4/2p5/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq -" => "d2d4",
        "rnbqkbnr/pp1ppppp/3p4/2p5/3PP3/5N2/PPP2PPP/RNBQKB1R b KQkq -" => "c5d4",
        "rnbqkbnr/pp1ppppp/3p4/8/3pP3/5N2/PPP2PPP/RNBQKB1R w KQkq -" => "f3d4",
        "rnbqkb1r/pp1ppppp/3p1n2/8/3NP3/8/PPP2PPP/RNBQKB1R w KQkq -" => "b1c3",
        "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq -" => "b8c6",
        "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq -" => "f1b5",
        "rnbqkbnr/ppp1pppp/8/3p4/3PP3/8/PPP2PPP/RNBQKBNR w KQkq -" => "c2c4",
        "rnbqkbnr/ppp1pppp/8/3p4/2PPP3/8/PP3PPP/RNBQKBNR b KQkq -" => "e7e6",
        "rnbqkbnr/ppp2ppp/4p3/3p4/2PPP3/8/PP3PPP/RNBQKBNR w KQkq -" => "b1c3",
        "rnbqkbnr/pppp1ppp/8/4p3/2P5/8/PP1PPPPP/RNBQKBNR w KQkq -" => "b1c3",
        "rnbqkb1r/pppp1ppp/5n2/4p3/2P5/2N5/PP1PPPPP/R1BQKBNR w KQkq -" => "g1f3",
        "rnbqkbnr/ppp1pppp/8/3p4/3P4/5N2/PPP1PPPP/RNBQKB1R b KQkq -" => "g8f6",
        _ => return None,
    };

    Some(book_move.parse().expect("opening book moves must be valid"))
}

fn opening_book_key(game: &Game) -> String {
    let fen = game.to_fen();
    let fields: Vec<&str> = fen.split_whitespace().collect();
    format!("{} {} {} -", fields[0], fields[1], fields[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluation_prefers_white_when_white_has_extra_queen() {
        let game = Game::from_fen("4k3/8/8/8/8/8/4Q3/4K3 w - - 0 1").expect("valid test FEN");
        assert!(game.evaluate() > 800);
    }

    #[test]
    fn search_finds_fools_mate_in_one() {
        let mut game = Game::new();
        for value in ["f2f3", "e7e5", "g2g4"] {
            game.make_move_str(value).expect("move should be legal");
        }

        let result = game.best_move(1);
        assert_eq!(result.best_move, Some("d8h4".parse().expect("valid move")));
        assert!(result.score > 29_000);
    }

    #[test]
    fn parallel_root_search_matches_sequential_result() {
        let mut game = Game::new();
        for value in ["e2e4", "c7c5", "g1f3", "d7d6", "d2d4", "c5d4"] {
            game.make_move_str(value).expect("move should be legal");
        }

        let depth = 4;
        let sequential_result = game.best_move_with_config(SearchConfig {
            depth,
            parallelism: Some(1),
            ..SearchConfig::default()
        });
        let parallel_result = game.best_move_with_config(SearchConfig {
            depth,
            parallelism: Some(2),
            ..SearchConfig::default()
        });

        assert_eq!(parallel_result.best_move, sequential_result.best_move);
    }

    #[test]
    fn opening_book_plays_the_first_move_instantly() {
        let game = Game::new();
        let result = game.best_move(10);

        assert_eq!(result.best_move, Some("e2e4".parse().expect("valid move")));
        assert_eq!(result.nodes, 0);
    }

    #[test]
    fn opening_book_replies_to_common_e4_with_sicilian() {
        let mut game = Game::new();
        game.make_move_str("e2e4").expect("move should be legal");

        let result = game.best_move(10);

        assert_eq!(result.best_move, Some("c7c5".parse().expect("valid move")));
        assert_eq!(result.nodes, 0);
    }

    #[test]
    fn search_falls_back_after_leaving_the_book() {
        let mut game = Game::new();
        for value in ["a2a3", "h7h6"] {
            game.make_move_str(value).expect("move should be legal");
        }

        let result = game.best_move(4);

        assert!(result.best_move.is_some());
        assert!(result.nodes > 0);
    }

    #[test]
    fn zero_time_limit_returns_a_legal_fallback_move() {
        let mut game = Game::new();
        for value in ["a2a3", "h7h6"] {
            game.make_move_str(value).expect("move should be legal");
        }

        let result = game.best_move_with_config(SearchConfig {
            depth: 8,
            time_limit: Some(Duration::ZERO),
            ..SearchConfig::default()
        });

        assert!(result.best_move.is_some());
        assert_eq!(result.nodes, 0);
    }

    #[test]
    fn search_handles_pawn_only_endgame_without_crashing() {
        let game = Game::from_fen("8/8/8/3k4/8/3P4/4K3/8 w - - 0 1").expect("valid test FEN");
        let result = game.best_move(5);

        assert!(result.best_move.is_some());
    }

    #[test]
    fn quiescence_finds_the_hanging_queen_capture() {
        let mut game =
            Game::from_fen("3rk3/8/8/8/8/8/3Q4/4K3 b - - 0 1").expect("valid test FEN");
        let static_eval = Searcher::default().evaluate_for_side_to_move(&game);
        let quiescence_eval =
            Searcher::default().quiescence(&mut game, 0, -INF, INF).expect("search should finish");

        assert!(quiescence_eval > static_eval + 300);
    }

    #[test]
    fn lock_free_transposition_table_clones_share_writes() {
        let table = TranspositionTable::new(32);
        let first = TranspositionEntry {
            key: 3,
            depth: 2,
            score: 17,
            bound: Bound::Exact,
            best_move: Some("e2e4".parse().expect("valid move")),
        };
        let second = TranspositionEntry {
            key: 7,
            depth: 3,
            score: 29,
            bound: Bound::Lower,
            best_move: Some("d2d4".parse().expect("valid move")),
        };

        table.store(first);

        let worker_view = table.clone();
        worker_view.store(second);

        assert_eq!(table.get(first.key), Some(first));
        assert_eq!(table.get(second.key), Some(second));
    }
}
