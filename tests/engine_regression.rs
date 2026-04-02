use chess_engine::{Color, Game, GameStatus, Move, Piece, PieceKind, Square};

const ALL_PIECE_KINDS: [PieceKind; 6] = [
    PieceKind::Pawn,
    PieceKind::Knight,
    PieceKind::Bishop,
    PieceKind::Rook,
    PieceKind::Queen,
    PieceKind::King,
];

#[derive(Clone, Copy, Debug)]
enum Policy {
    Search {
        depth: u8,
    },
    MonteCarlo {
        playouts: usize,
        rollout_depth: usize,
    },
}

#[derive(Clone, Debug)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn gen_index(&mut self, upper_bound: usize) -> usize {
        debug_assert!(upper_bound > 0);
        (self.next_u64() % upper_bound as u64) as usize
    }
}

fn play_moves(moves: &[&str]) -> Game {
    let mut game = Game::new();
    for chess_move in moves {
        game.make_move_str(chess_move)
            .unwrap_or_else(|error| panic!("move {chess_move} should be legal: {error}"));
    }
    game
}

fn square(name: &str) -> Square {
    name.parse().expect("square should parse")
}

fn assert_game_invariants(game: &Game) {
    let fen = game.to_fen();
    let round_trip = Game::from_fen(&fen).expect("game FEN should round-trip");
    assert_eq!(round_trip.to_fen(), fen);

    let mut total_pieces = 0_u32;
    let mut white_kings = 0_u32;
    let mut black_kings = 0_u32;

    for rank in 0..8 {
        for file in 0..8 {
            let square = Square::from_coords(file, rank).expect("board square should exist");
            if let Some(piece) = game.board().piece_at(square) {
                total_pieces += 1;
                match piece {
                    Piece {
                        color: Color::White,
                        kind: PieceKind::King,
                    } => white_kings += 1,
                    Piece {
                        color: Color::Black,
                        kind: PieceKind::King,
                    } => black_kings += 1,
                    _ => {}
                }
            }
        }
    }

    assert_eq!(white_kings, 1, "game should always contain one white king");
    assert_eq!(black_kings, 1, "game should always contain one black king");
    assert!(
        total_pieces <= 32,
        "piece count should never exceed a full board"
    );

    let bitboard_piece_count: u32 = [Color::White, Color::Black]
        .into_iter()
        .flat_map(|color| {
            ALL_PIECE_KINDS
                .into_iter()
                .map(move |kind| game.board().bitboard(color, kind).count_ones())
        })
        .sum();
    assert_eq!(bitboard_piece_count, total_pieces);
    assert_eq!(game.board().occupancy(None).count_ones(), total_pieces);

    let legal_moves = game.legal_moves();
    match game.status() {
        GameStatus::Ongoing => assert!(
            !legal_moves.is_empty(),
            "ongoing positions should expose at least one legal move"
        ),
        GameStatus::Checkmate { winner } => {
            assert!(
                legal_moves.is_empty(),
                "checkmate positions should have no legal moves"
            );
            assert!(game.is_in_check(game.side_to_move()));
            assert_ne!(winner, game.side_to_move());
        }
        GameStatus::Stalemate => {
            assert!(
                legal_moves.is_empty(),
                "stalemate positions should have no legal moves"
            );
            assert!(!game.is_in_check(game.side_to_move()));
        }
    }
}

fn select_policy_move(game: &Game, policy: Policy, rng: &mut DeterministicRng) -> Option<Move> {
    match policy {
        Policy::Search { depth } => game.best_move(depth).best_move,
        Policy::MonteCarlo {
            playouts,
            rollout_depth,
        } => monte_carlo_best_move(game, playouts, rollout_depth, rng),
    }
}

fn monte_carlo_best_move(
    game: &Game,
    playouts: usize,
    rollout_depth: usize,
    rng: &mut DeterministicRng,
) -> Option<Move> {
    let root_color = game.side_to_move();
    let mut legal_moves = game.legal_moves();
    legal_moves.sort_by_key(|candidate| candidate.to_uci());

    let mut best_move = None;
    let mut best_score = f32::NEG_INFINITY;

    for candidate in legal_moves {
        let mut total_score = 0.0_f32;

        for _ in 0..playouts.max(1) {
            let mut next = game.clone();
            next.make_move(candidate)
                .expect("monte carlo candidate should be legal");
            total_score += rollout_score(&next, root_color, rollout_depth, rng);
        }

        let average_score = total_score / playouts.max(1) as f32;
        if average_score > best_score + f32::EPSILON {
            best_score = average_score;
            best_move = Some(candidate);
        }
    }

    best_move
}

fn rollout_score(
    game: &Game,
    root_color: Color,
    rollout_depth: usize,
    rng: &mut DeterministicRng,
) -> f32 {
    let mut state = game.clone();

    for _ in 0..rollout_depth {
        match state.status() {
            GameStatus::Checkmate { winner } => {
                return if winner == root_color { 1.0 } else { -1.0 };
            }
            GameStatus::Stalemate => return 0.0,
            GameStatus::Ongoing => {}
        }

        let legal_moves = state.legal_moves();
        if legal_moves.is_empty() {
            break;
        }

        let index = rng.gen_index(legal_moves.len());
        state
            .make_move(legal_moves[index])
            .expect("rollout move should be legal");
    }

    let white_perspective_score = state.evaluate();
    let root_score = if root_color == Color::White {
        white_perspective_score
    } else {
        -white_perspective_score
    };
    (root_score as f32 / 1_000.0).clamp(-0.95, 0.95)
}

fn play_match(white: Policy, black: Policy, max_plies: usize, seed: u64) -> Game {
    let mut game = Game::new();
    let mut rng = DeterministicRng::new(seed);
    assert_game_invariants(&game);

    for _ in 0..max_plies {
        if game.status() != GameStatus::Ongoing {
            break;
        }

        let policy = match game.side_to_move() {
            Color::White => white,
            Color::Black => black,
        };
        let selected_move = select_policy_move(&game, policy, &mut rng)
            .expect("ongoing positions should produce at least one move");

        assert!(
            game.legal_moves().contains(&selected_move),
            "selected policy move must be legal"
        );

        game.make_move(selected_move)
            .expect("selected policy move should apply cleanly");
        assert_game_invariants(&game);
    }

    game
}

#[test]
fn scholars_mate_sequence_ends_in_white_checkmate() {
    let game = play_moves(&["e2e4", "e7e5", "d1h5", "b8c6", "f1c4", "g8f6", "h5f7"]);

    assert_eq!(
        game.status(),
        GameStatus::Checkmate {
            winner: Color::White,
        }
    );
    assert!(game.is_in_check(Color::Black));
    assert!(game.legal_moves().is_empty());
}

#[test]
fn castling_sequence_moves_kings_and_rooks_to_their_castled_squares() {
    let game = play_moves(&[
        "e2e4", "e7e5", "g1f3", "b8c6", "f1e2", "g8f6", "e1g1", "f8e7", "d2d3", "e8g8",
    ]);

    assert_eq!(
        game.board().piece_at(square("g1")),
        Some(Piece::new(Color::White, PieceKind::King))
    );
    assert_eq!(
        game.board().piece_at(square("f1")),
        Some(Piece::new(Color::White, PieceKind::Rook))
    );
    assert_eq!(
        game.board().piece_at(square("g8")),
        Some(Piece::new(Color::Black, PieceKind::King))
    );
    assert_eq!(
        game.board().piece_at(square("f8")),
        Some(Piece::new(Color::Black, PieceKind::Rook))
    );

    let rights = game.castling_rights();
    assert!(!rights.white_king_side);
    assert!(!rights.white_queen_side);
    assert!(!rights.black_king_side);
    assert!(!rights.black_queen_side);
    assert_game_invariants(&game);
}

#[test]
fn promotion_move_creates_the_requested_piece() {
    let mut game = Game::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").expect("test FEN should parse");
    game.make_move_str("a7a8n")
        .expect("under-promotion move should be legal");

    assert_eq!(
        game.board().piece_at(square("a8")),
        Some(Piece::new(Color::White, PieceKind::Knight))
    );
    assert_eq!(game.board().piece_at(square("a7")), None);
    assert_game_invariants(&game);
}

#[test]
fn stalemate_position_is_reported_correctly() {
    let game = Game::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").expect("test FEN should parse");

    assert_eq!(game.status(), GameStatus::Stalemate);
    assert!(!game.is_in_check(Color::Black));
    assert!(game.legal_moves().is_empty());
}

#[test]
fn double_pawn_push_sets_en_passant_target_until_the_reply() {
    let mut game = Game::new();
    game.make_move_str("e2e4")
        .expect("opening pawn push should be legal");
    assert_eq!(game.en_passant_target(), Some(square("e3")));

    game.make_move_str("a7a6")
        .expect("reply move should be legal");
    assert_eq!(game.en_passant_target(), None);
    assert_game_invariants(&game);
}

#[test]
fn search_self_play_stays_legal_for_a_fixed_number_of_plies() {
    let game = play_match(
        Policy::Search { depth: 2 },
        Policy::Search { depth: 2 },
        12,
        0x00c0_ffee,
    );

    assert!(game.fullmove_number() >= 4);
    assert_game_invariants(&game);
}

#[test]
fn search_vs_monte_carlo_match_stays_legal_for_a_fixed_number_of_plies() {
    let game = play_match(
        Policy::Search { depth: 2 },
        Policy::MonteCarlo {
            playouts: 3,
            rollout_depth: 6,
        },
        10,
        0xdead_beef,
    );

    assert!(game.fullmove_number() >= 3);
    assert_game_invariants(&game);
}

#[test]
fn monte_carlo_policy_finds_the_forced_mate_in_one() {
    let game = play_moves(&["f2f3", "e7e5", "g2g4"]);
    let mut rng = DeterministicRng::new(0x1234_5678);

    let selected_move = monte_carlo_best_move(&game, 4, 4, &mut rng);
    assert_eq!(
        selected_move,
        Some("d8h4".parse().expect("move should parse"))
    );
}
