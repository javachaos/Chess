#[cfg(not(target_arch = "wasm32"))]
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use crate::{
    game::{Game, GameStatus},
    search::{SearchConfig, SearchResult},
    types::{Color, Move, Piece, PieceKind, Square},
};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Stroke, StrokeKind, Vec2};
use web_time::{Duration, Instant};

const BOARD_SQUARE_SIZE: f32 = 64.0;
const BOARD_LABEL_SIZE: f32 = 20.0;
const BOARD_FRAME_PADDING: i8 = 12;
const BOARD_FRAME_STROKE: f32 = 3.0;
const PIECE_FONT_SIZE: f32 = 42.0;

pub struct ChessApp {
    game: Game,
    human_color: Color,
    ai_depth: u8,
    ai_time_limit_seconds: f32,
    selected_square: Option<Square>,
    last_move: Option<Move>,
    pending_ai_turn: bool,
    ai_task: Option<AiTask>,
    message: String,
    last_ai_result: Option<SearchResult>,
    resigned_winner: Option<Color>,
}

#[cfg(not(target_arch = "wasm32"))]
struct AiTask {
    started_at: Instant,
    depth: u8,
    time_limit: Duration,
    receiver: Receiver<SearchResult>,
}

#[cfg(target_arch = "wasm32")]
struct AiTask {
    started_at: Instant,
    depth: u8,
    time_limit: Duration,
    position: Game,
    primed: bool,
}

impl ChessApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);

        Self {
            game: Game::new(),
            human_color: Color::White,
            ai_depth: 2,
            ai_time_limit_seconds: 3.0,
            selected_square: None,
            last_move: None,
            pending_ai_turn: false,
            ai_task: None,
            message: "Select a piece to move. The AI replies automatically.".to_string(),
            last_ai_result: None,
            resigned_winner: None,
        }
    }

    fn reset_game(&mut self, human_color: Color) {
        self.game = Game::new();
        self.human_color = human_color;
        self.selected_square = None;
        self.last_move = None;
        self.last_ai_result = None;
        self.ai_task = None;
        self.message = format!("{human_color} is yours. Start playing.");
        self.pending_ai_turn = self.game.side_to_move() != self.human_color;
        self.resigned_winner = None;
    }

    fn is_game_over(&self) -> bool {
        self.resigned_winner.is_some() || self.game.status() != GameStatus::Ongoing
    }

    fn resign(&mut self) {
        if self.is_game_over() {
            return;
        }

        let winner = self.human_color.opposite();
        self.resigned_winner = Some(winner);
        self.pending_ai_turn = false;
        self.ai_task = None;
        self.selected_square = None;
        self.last_ai_result = None;
        self.message = format!("You resigned. {winner} wins.");
    }

    fn maybe_start_ai_turn(&mut self) {
        if self.resigned_winner.is_some() || self.ai_task.is_some() || !self.pending_ai_turn {
            return;
        }
        if self.game.status() != GameStatus::Ongoing {
            self.pending_ai_turn = false;
            return;
        }
        if self.game.side_to_move() == self.human_color {
            self.pending_ai_turn = false;
            return;
        }

        let depth = self.ai_depth;
        let time_limit = Duration::from_secs_f32(self.ai_time_limit_seconds);
        let position = self.game.clone();

        self.pending_ai_turn = false;
        self.last_ai_result = None;
        self.selected_square = None;
        self.message = format!(
            "AI is thinking up to depth {depth} with a {:.1}s limit...",
            time_limit.as_secs_f32()
        );

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (sender, receiver) = mpsc::channel();
            self.ai_task = Some(AiTask {
                started_at: Instant::now(),
                depth,
                time_limit,
                receiver,
            });

            thread::spawn(move || {
                let result = position.best_move_with_config(SearchConfig {
                    depth,
                    time_limit: Some(time_limit),
                    ..SearchConfig::default()
                });
                let _ = sender.send(result);
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.ai_task = Some(AiTask {
                started_at: Instant::now(),
                depth,
                time_limit,
                position,
                primed: false,
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn poll_ai_turn(&mut self, ctx: &egui::Context) {
        let state = self
            .ai_task
            .as_mut()
            .map(|task| match task.receiver.try_recv() {
                Ok(result) => PollState::Finished {
                    result,
                    elapsed: task.started_at.elapsed(),
                },
                Err(TryRecvError::Empty) => PollState::Pending,
                Err(TryRecvError::Disconnected) => PollState::Disconnected,
            });

        self.handle_ai_poll_state(state, ctx);
    }

    #[cfg(target_arch = "wasm32")]
    fn poll_ai_turn(&mut self, ctx: &egui::Context) {
        let state = self.ai_task.as_mut().map(|task| {
            if !task.primed {
                task.primed = true;
                PollState::Pending
            } else {
                let result = task.position.best_move_with_config(SearchConfig {
                    depth: task.depth,
                    time_limit: Some(task.time_limit),
                    ..SearchConfig::default()
                });
                PollState::Finished {
                    result,
                    elapsed: task.started_at.elapsed(),
                }
            }
        });

        self.handle_ai_poll_state(state, ctx);
    }

    fn handle_ai_poll_state(&mut self, state: Option<PollState>, ctx: &egui::Context) {
        match state {
            Some(PollState::Finished { result, elapsed }) => {
                self.ai_task = None;
                self.last_ai_result = Some(result);

                if let Some(best_move) = result.best_move {
                    self.game
                        .make_move(best_move)
                        .expect("search should only return legal moves");
                    self.last_move = Some(best_move);
                    self.selected_square = None;
                    self.message = format!(
                        "AI played {} at depth {} (score {}, nodes {}) in {:.1}s.",
                        best_move,
                        result.depth,
                        result.score,
                        result.nodes,
                        elapsed.as_secs_f32()
                    );
                } else {
                    self.message = "AI found no legal move.".to_string();
                }

                ctx.request_repaint();
            }
            Some(PollState::Pending) => {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            #[cfg(not(target_arch = "wasm32"))]
            Some(PollState::Disconnected) => {
                self.ai_task = None;
                self.message = "AI search stopped unexpectedly.".to_string();
                ctx.request_repaint();
            }
            None => {}
        }
    }

    fn ai_thinking_text(&self) -> Option<String> {
        self.ai_task.as_ref().map(|task| {
            let dots = match ((task.started_at.elapsed().as_secs_f32() * 3.0) as usize) % 4 {
                0 => "",
                1 => ".",
                2 => "..",
                _ => "...",
            };
            format!(
                "AI thinking up to depth {} ({:.1}s max){}",
                task.depth,
                task.time_limit.as_secs_f32(),
                dots
            )
        })
    }

    fn ai_elapsed(&self) -> Option<f32> {
        self.ai_task
            .as_ref()
            .map(|task| task.started_at.elapsed().as_secs_f32())
    }

    fn ai_progress(&self) -> Option<f32> {
        self.ai_task.as_ref().map(|task| {
            (task.started_at.elapsed().as_secs_f32() / task.time_limit.as_secs_f32())
                .clamp(0.0, 1.0)
        })
    }

    fn status_text(&self) -> String {
        if let Some(winner) = self.resigned_winner {
            return format!("{winner} wins by resignation.");
        }

        match self.game.status() {
            GameStatus::Ongoing => {
                if self.game.is_in_check(self.game.side_to_move()) {
                    format!("{} to move and in check.", self.game.side_to_move())
                } else {
                    format!("{} to move.", self.game.side_to_move())
                }
            }
            GameStatus::Checkmate { winner } => format!("Checkmate. {winner} wins."),
            GameStatus::Stalemate => "Stalemate.".to_string(),
        }
    }

    fn game_over_overlay(&self) -> Option<(String, String, Color32)> {
        if let Some(winner) = self.resigned_winner {
            return Some((
                "Resignation".to_string(),
                format!("{winner} wins."),
                Color32::from_rgb(214, 193, 149),
            ));
        }

        match self.game.status() {
            GameStatus::Ongoing => None,
            GameStatus::Checkmate { winner } => Some((
                "Checkmate".to_string(),
                format!("{winner} wins."),
                Color32::from_rgb(214, 193, 149),
            )),
            GameStatus::Stalemate => Some((
                "Stalemate".to_string(),
                "No legal moves remain.".to_string(),
                Color32::from_rgb(166, 188, 204),
            )),
        }
    }

    fn board_piece_label(piece: Piece) -> &'static str {
        match (piece.color, piece.kind) {
            (Color::White, PieceKind::Pawn) => "♙",
            (Color::White, PieceKind::Knight) => "♘",
            (Color::White, PieceKind::Bishop) => "♗",
            (Color::White, PieceKind::Rook) => "♖",
            (Color::White, PieceKind::Queen) => "♕",
            (Color::White, PieceKind::King) => "♔",
            (Color::Black, PieceKind::Pawn) => "♟",
            (Color::Black, PieceKind::Knight) => "♞",
            (Color::Black, PieceKind::Bishop) => "♝",
            (Color::Black, PieceKind::Rook) => "♜",
            (Color::Black, PieceKind::Queen) => "♛",
            (Color::Black, PieceKind::King) => "♚",
        }
    }

    fn displayed_files(&self) -> [u8; 8] {
        match self.human_color {
            Color::White => [0, 1, 2, 3, 4, 5, 6, 7],
            Color::Black => [7, 6, 5, 4, 3, 2, 1, 0],
        }
    }

    fn displayed_ranks(&self) -> [u8; 8] {
        match self.human_color {
            Color::White => [7, 6, 5, 4, 3, 2, 1, 0],
            Color::Black => [0, 1, 2, 3, 4, 5, 6, 7],
        }
    }

    fn legal_moves(&self) -> Vec<Move> {
        self.game.legal_moves()
    }

    fn legal_moves_from_selected(&self, legal_moves: &[Move]) -> Vec<Move> {
        self.selected_square.map_or_else(Vec::new, |selected| {
            legal_moves
                .iter()
                .copied()
                .filter(|candidate| candidate.from == selected)
                .collect()
        })
    }

    fn resolve_click_move(&self, from: Square, to: Square, legal_moves: &[Move]) -> Option<Move> {
        let mut candidates: Vec<Move> = legal_moves
            .iter()
            .copied()
            .filter(|candidate| candidate.from == from && candidate.to == to)
            .collect();

        if candidates.is_empty() {
            return None;
        }

        candidates.sort_by_key(|candidate| {
            candidate
                .promotion
                .map(|promotion| match promotion {
                    PieceKind::Queen => 0,
                    PieceKind::Knight => 1,
                    PieceKind::Rook => 2,
                    PieceKind::Bishop => 3,
                    PieceKind::Pawn | PieceKind::King => 4,
                })
                .unwrap_or(0)
        });
        candidates.into_iter().next()
    }

    fn click_square(&mut self, square: Square, legal_moves: &[Move], ctx: &egui::Context) {
        if self.is_game_over() || self.game.side_to_move() != self.human_color {
            return;
        }

        if let Some(selected) = self.selected_square {
            if selected == square {
                self.selected_square = None;
                self.message = "Selection cleared.".to_string();
                return;
            }

            if let Some(chess_move) = self.resolve_click_move(selected, square, legal_moves) {
                self.game
                    .make_move(chess_move)
                    .expect("selected move should always be legal");
                self.last_move = Some(chess_move);
                self.selected_square = None;
                self.last_ai_result = None;
                self.pending_ai_turn = self.game.status() == GameStatus::Ongoing
                    && self.game.side_to_move() != self.human_color;
                self.message = if chess_move.promotion.is_some() {
                    format!(
                        "You played {}. Promotions default to the strongest available choice.",
                        chess_move
                    )
                } else {
                    format!("You played {}.", chess_move)
                };
                ctx.request_repaint();
                return;
            }
        }

        match self.game.board().piece_at(square) {
            Some(piece)
                if piece.color == self.human_color && piece.color == self.game.side_to_move() =>
            {
                self.selected_square = Some(square);
                self.message = format!("Selected {}.", square);
            }
            _ => {
                self.selected_square = None;
            }
        }
    }

    fn draw_board(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let files = self.displayed_files();
        let ranks = self.displayed_ranks();
        let legal_moves = self.legal_moves();
        let selected_moves = self.legal_moves_from_selected(&legal_moves);

        let board_inset = BOARD_FRAME_STROKE + f32::from(BOARD_FRAME_PADDING);

        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.add_space(board_inset);
                    for rank in ranks {
                        draw_board_label(
                            ui,
                            Vec2::new(BOARD_LABEL_SIZE, BOARD_SQUARE_SIZE),
                            format!("{}", rank + 1),
                        );
                    }
                });

                let board_frame = egui::Frame::default()
                    .fill(Color32::from_rgb(108, 108, 112))
                    .stroke(Stroke::new(
                        BOARD_FRAME_STROKE,
                        Color32::from_rgb(58, 58, 62),
                    ))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::same(BOARD_FRAME_PADDING))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        ui.vertical(|ui| {
                            for rank in ranks {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                                    for file in files {
                                        let square = Square::from_coords(file, rank)
                                            .expect("board coordinates should exist");
                                        let piece = self.game.board().piece_at(square);
                                        let is_selected = self.selected_square == Some(square);
                                        let is_legal_target = selected_moves
                                            .iter()
                                            .any(|candidate| candidate.to == square);
                                        let is_last_move_square =
                                            self.last_move.is_some_and(|chess_move| {
                                                chess_move.from == square
                                                    || chess_move.to == square
                                            });

                                        let fill = square_fill(
                                            square,
                                            is_selected,
                                            is_legal_target,
                                            is_last_move_square,
                                        );
                                        let button = egui::Button::new(RichText::new(" "))
                                            .min_size(Vec2::splat(BOARD_SQUARE_SIZE))
                                            .fill(fill)
                                            .stroke(Stroke::NONE)
                                            .corner_radius(egui::CornerRadius::ZERO);

                                        let response = ui.add(button);
                                        if is_selected {
                                            ui.painter().rect_stroke(
                                                response.rect.shrink(1.0),
                                                egui::CornerRadius::ZERO,
                                                Stroke::new(
                                                    2.0,
                                                    Color32::from_rgb(180, 134, 24),
                                                ),
                                                StrokeKind::Inside,
                                            );
                                        }
                                        if let Some(piece) = piece {
                                            paint_piece(ui, response.rect, piece);
                                        }

                                        if response.clicked() {
                                            self.click_square(square, &legal_moves, ctx);
                                        }

                                        if let Some(piece) = piece {
                                            response.on_hover_text(format!(
                                                "{} {} on {}",
                                                piece.color,
                                                piece_name(piece.kind),
                                                square
                                            ));
                                        } else {
                                            response.on_hover_text(square.to_string());
                                        }
                                    }
                                });
                            }
                        });
                    });

                if let Some((title, subtitle, accent)) = self.game_over_overlay() {
                    paint_game_over_overlay(
                        ui,
                        board_frame.response.rect.shrink(16.0),
                        &title,
                        &subtitle,
                        accent,
                    );
                }
            });

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.add_space(BOARD_LABEL_SIZE + board_inset);
                for file in files {
                    draw_board_label(
                        ui,
                        Vec2::new(BOARD_SQUARE_SIZE, 16.0),
                        ((b'a' + file) as char).to_string(),
                    );
                }
            });
        });
    }
}

impl eframe::App for ChessApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.maybe_start_ai_turn();
        self.poll_ai_turn(ctx);

        egui::SidePanel::right("controls")
            .resizable(false)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("Play vs AI");
                ui.label(self.status_text());
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("You play:");
                    let mut selected_color = self.human_color;
                    ui.selectable_value(&mut selected_color, Color::White, "White");
                    ui.selectable_value(&mut selected_color, Color::Black, "Black");
                    if selected_color != self.human_color {
                        self.reset_game(selected_color);
                        ctx.request_repaint();
                    }
                });

                ui.add(egui::Slider::new(&mut self.ai_depth, 1..=15).text("AI depth"));
                ui.add(
                    egui::Slider::new(&mut self.ai_time_limit_seconds, 0.25..=15.0)
                        .text("Time / move (s)")
                        .logarithmic(true),
                );
                ui.label("Depth is a ceiling. The AI stops early when it hits the time limit.");

                #[cfg(target_arch = "wasm32")]
                ui.small("Browser builds search on one thread, so higher depths may feel heavier.");

                #[cfg(not(target_arch = "wasm32"))]
                if cfg!(debug_assertions) {
                    ui.add_space(6.0);
                    ui.group(|ui| {
                        ui.label(
                            RichText::new("Debug build")
                                .strong()
                                .color(Color32::from_rgb(230, 197, 118)),
                        );
                        ui.label("AI search is much slower in debug mode.");
                        ui.monospace("Use `cargo run --release` for normal play.");
                    });
                }

                ui.horizontal(|ui| {
                    if ui.button("New Game").clicked() {
                        self.reset_game(self.human_color);
                        ctx.request_repaint();
                    }

                    let resign_button = ui.add_enabled(
                        !self.is_game_over(),
                        egui::Button::new(
                            RichText::new("Resign").color(Color32::from_rgb(240, 214, 214)),
                        ),
                    );
                    if resign_button.clicked() {
                        self.resign();
                        ctx.request_repaint();
                    }
                });

                ui.separator();
                ui.label(RichText::new("Status").strong());
                ui.label(&self.message);

                if let Some(text) = self.ai_thinking_text() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(18.0));
                        ui.label(text);
                    });
                    ui.add(
                        egui::ProgressBar::new(self.ai_progress().unwrap_or_default())
                            .desired_width(ui.available_width())
                            .text(format!(
                                "{:.1}/{:.1}s",
                                self.ai_elapsed().unwrap_or_default(),
                                self.ai_task
                                    .as_ref()
                                    .map(|task| task.time_limit.as_secs_f32())
                                    .unwrap_or_default()
                            )),
                    );
                    if let Some(elapsed) = self.ai_elapsed() {
                        ui.small(format!("Elapsed: {:.1}s", elapsed));
                    }
                }

                if let Some(result) = self.last_ai_result {
                    if let Some(best_move) = result.best_move {
                        ui.label(format!(
                            "Last AI move: {} (depth {}, score {}, nodes {})",
                            best_move, result.depth, result.score, result.nodes
                        ));
                    }
                }

                ui.separator();
                ui.label(RichText::new("Position").strong());
                ui.monospace(format!("FEN: {}", self.game.to_fen()));
                ui.label(format!("Static eval: {}", self.game.evaluate()));
                ui.label(format!("Fullmove: {}", self.game.fullmove_number()));
                ui.label(format!("Halfmove clock: {}", self.game.halfmove_clock()));

                if let Some(selected) = self.selected_square {
                    ui.separator();
                    ui.label(RichText::new("Selection").strong());
                    ui.label(format!("Selected square: {}", selected));
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Chess Board");
            ui.label("Click one of your pieces, then click a highlighted target square.");
            ui.add_space(12.0);
            self.draw_board(ui, ctx);
        });
    }
}

enum PollState {
    Finished {
        result: SearchResult,
        elapsed: Duration,
    },
    Pending,
    #[cfg(not(target_arch = "wasm32"))]
    Disconnected,
}

fn square_fill(
    square: Square,
    is_selected: bool,
    is_legal_target: bool,
    is_last_move_square: bool,
) -> Color32 {
    if is_selected {
        return Color32::from_rgb(246, 212, 120);
    }
    if is_legal_target {
        return Color32::from_rgb(164, 209, 133);
    }
    if is_last_move_square {
        return Color32::from_rgb(178, 208, 233);
    }

    if (square.file() + square.rank()) % 2 == 0 {
        Color32::from_rgb(206, 186, 150)
    } else {
        Color32::from_rgb(123, 92, 67)
    }
}

fn piece_name(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::Pawn => "pawn",
        PieceKind::Knight => "knight",
        PieceKind::Bishop => "bishop",
        PieceKind::Rook => "rook",
        PieceKind::Queen => "queen",
        PieceKind::King => "king",
    }
}

fn draw_board_label(ui: &mut egui::Ui, size: Vec2, text: String) {
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        |ui| {
            ui.label(RichText::new(text).strong());
        },
    );
}

fn paint_game_over_overlay(
    ui: &egui::Ui,
    rect: egui::Rect,
    title: &str,
    subtitle: &str,
    accent: Color32,
) {
    let overlay_fill = Color32::from_rgba_premultiplied(18, 19, 16, 224);
    let painter = ui.painter();
    let title_pos = rect.center_top() + Vec2::new(0.0, rect.height() * 0.24);
    let subtitle_pos = rect.center_top() + Vec2::new(0.0, rect.height() * 0.48);
    let hint_pos = rect.center_top() + Vec2::new(0.0, rect.height() * 0.66);

    painter.rect_filled(rect, egui::CornerRadius::same(18), overlay_fill);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(18),
        Stroke::new(2.0, accent),
        StrokeKind::Inside,
    );
    painter.text(
        title_pos,
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(36.0),
        Color32::from_rgb(246, 240, 226),
    );
    painter.text(
        subtitle_pos,
        Align2::CENTER_CENTER,
        subtitle,
        FontId::proportional(22.0),
        accent,
    );
    painter.text(
        hint_pos,
        Align2::CENTER_CENTER,
        "Start a new game from the side panel.",
        FontId::proportional(16.0),
        Color32::from_rgb(210, 203, 191),
    );
}

fn paint_piece(ui: &egui::Ui, rect: egui::Rect, piece: Piece) {
    let glyph = ChessApp::board_piece_label(piece);
    let (fill, outline) = match piece.color {
        Color::White => (
            Color32::from_rgb(248, 246, 240),
            Color32::from_rgb(20, 20, 20),
        ),
        Color::Black => (
            Color32::from_rgb(26, 26, 26),
            Color32::from_rgb(240, 240, 240),
        ),
    };
    let font = FontId::proportional(PIECE_FONT_SIZE);
    let center = rect.center();
    let painter = ui.painter();

    for offset in [
        Vec2::new(-1.2, 0.0),
        Vec2::new(1.2, 0.0),
        Vec2::new(0.0, -1.2),
        Vec2::new(0.0, 1.2),
        Vec2::new(-1.2, -1.2),
        Vec2::new(1.2, -1.2),
        Vec2::new(-1.2, 1.2),
        Vec2::new(1.2, 1.2),
    ] {
        painter.text(
            center + offset,
            Align2::CENTER_CENTER,
            glyph,
            font.clone(),
            outline,
        );
    }

    painter.text(center, Align2::CENTER_CENTER, glyph, font, fill);
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(Color32::from_rgb(231, 223, 210));
    visuals.panel_fill = Color32::from_rgb(30, 31, 27);
    visuals.window_fill = Color32::from_rgb(36, 37, 33);
    visuals.faint_bg_color = Color32::from_rgb(47, 47, 42);
    visuals.extreme_bg_color = Color32::from_rgb(20, 21, 18);
    visuals.code_bg_color = Color32::from_rgb(26, 27, 24);
    visuals.hyperlink_color = Color32::from_rgb(164, 198, 214);
    visuals.selection.bg_fill = Color32::from_rgb(121, 147, 84);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(243, 235, 219));
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(44, 45, 40);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(72, 72, 64));
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(66, 63, 56);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(95, 91, 80));
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(92, 88, 78);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(171, 158, 133));
    visuals.widgets.active.bg_fill = Color32::from_rgb(121, 116, 102);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(214, 193, 149));
    visuals.widgets.open.bg_fill = Color32::from_rgb(57, 56, 49);
    visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(74, 74, 66));

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.button_padding = Vec2::new(10.0, 7.0);
    style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.open.corner_radius = egui::CornerRadius::same(8);
    ctx.set_style(style);
}
