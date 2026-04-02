use std::io::{self, Write};

use chess_engine::{Game, GameStatus, Move};

fn main() {
    let mut game = Game::new();

    println!("Rust chess engine demo");
    println!(
        "Commands: board, fen, moves, eval, best [depth], playbest [depth], reset, quit, or a move like e2e4 / e7e8q"
    );
    println!("{}", game.board());

    loop {
        print!("{} to move> ", game.side_to_move());
        io::stdout().flush().expect("stdout flush should succeed");

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            eprintln!("failed to read input");
            continue;
        }

        let command = input.trim();
        if command.is_empty() {
            continue;
        }

        let parts: Vec<&str> = command.split_whitespace().collect();
        match parts.as_slice() {
            ["quit"] | ["exit"] => break,
            ["board"] => println!("{}", game.board()),
            ["fen"] => println!("{}", game.to_fen()),
            ["eval"] => {
                println!("Static evaluation: {}", game.evaluate());
            }
            ["best"] | ["best", ..] => {
                let depth = parse_depth(parts.get(1).copied()).unwrap_or(4);
                let result = game.best_move(depth);
                match result.best_move {
                    Some(best_move) => {
                        println!(
                            "Best move at depth {}: {} (score {}, nodes {})",
                            result.depth, best_move, result.score, result.nodes
                        );
                    }
                    None => println!("No legal moves available."),
                }
            }
            ["playbest"] | ["playbest", ..] => {
                let depth = parse_depth(parts.get(1).copied()).unwrap_or(4);
                let result = game.best_move(depth);
                match result.best_move {
                    Some(best_move) => {
                        println!(
                            "Playing {} (depth {}, score {}, nodes {})",
                            best_move, result.depth, result.score, result.nodes
                        );
                        game.make_move(best_move)
                            .expect("search should only return legal moves");
                        println!("{}", game.board());
                    }
                    None => println!("No legal moves available."),
                }
            }
            ["reset"] => {
                game = Game::new();
                println!("{}", game.board());
            }
            ["moves"] => {
                let mut legal_moves: Vec<String> =
                    game.legal_moves().into_iter().map(Move::to_uci).collect();
                legal_moves.sort();
                println!("{}", legal_moves.join(" "));
            }
            [value] => match game.make_move_str(value) {
                Ok(()) => {
                    println!("{}", game.board());
                    match game.status() {
                        GameStatus::Ongoing => {}
                        GameStatus::Checkmate { winner } => {
                            println!("Checkmate. {winner} wins.");
                        }
                        GameStatus::Stalemate => {
                            println!("Stalemate.");
                        }
                    }
                }
                Err(error) => {
                    eprintln!("{error}");
                }
            },
            _ => eprintln!("unknown command"),
        }
    }
}

fn parse_depth(value: Option<&str>) -> Option<u8> {
    value.and_then(|value| value.parse::<u8>().ok())
}
