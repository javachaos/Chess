use std::{
    env, thread,
    time::{Duration, Instant},
};

use chess_engine::{Game, SearchConfig};

fn middlegame_position() -> Game {
    let mut game = Game::new();
    for chess_move in [
        "e2e4", "c7c5", "g1f3", "d7d6", "d2d4", "c5d4", "f3d4", "g8f6", "b1c3", "a7a6",
    ] {
        game.make_move_str(chess_move)
            .unwrap_or_else(|error| panic!("move {chess_move} should be legal: {error}"));
    }
    game
}

fn timed<T>(label: &str, operation: impl FnOnce() -> T) -> (T, Duration) {
    let started_at = Instant::now();
    let result = operation();
    let elapsed = started_at.elapsed();
    println!("{label}: {:.3}s", elapsed.as_secs_f64());
    (result, elapsed)
}

fn budget_ms(env_key: &str, debug_default_ms: u128, release_default_ms: u128) -> u128 {
    env::var(env_key)
        .ok()
        .and_then(|value| value.parse::<u128>().ok())
        .unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                debug_default_ms
            } else {
                release_default_ms
            }
        })
}

fn assert_under_budget(elapsed: Duration, budget_ms: u128, operation_name: &str, env_key: &str) {
    let elapsed_ms = elapsed.as_millis();
    assert!(
        elapsed_ms <= budget_ms,
        "{operation_name} took {elapsed_ms}ms which exceeded the {budget_ms}ms budget. \
         Override with {env_key}=<ms> if you need a looser local threshold."
    );
}

#[test]
fn perft_depth_four_smoke_budget() {
    let game = Game::new();
    let (nodes, elapsed) = timed("perft depth 4", || game.perft(4));

    assert_eq!(nodes, 197_281);
    assert_under_budget(
        elapsed,
        budget_ms("CHESS_PERF_PERFT4_MS", 2_500, 700),
        "perft(4)",
        "CHESS_PERF_PERFT4_MS",
    );
}

#[test]
fn search_depth_three_smoke_budget() {
    let game = middlegame_position();
    let (result, elapsed) = timed("search depth 3", || {
        game.best_move_with_config(SearchConfig {
            depth: 3,
            parallelism: None,
            ..SearchConfig::default()
        })
    });

    assert!(
        result.best_move.is_some(),
        "search should return a legal move"
    );
    assert!(result.nodes > 0, "search should visit at least one node");
    assert_under_budget(
        elapsed,
        budget_ms("CHESS_PERF_SEARCH3_MS", 3_500, 1_200),
        "depth-3 search",
        "CHESS_PERF_SEARCH3_MS",
    );
}

#[test]
#[ignore = "machine-dependent performance benchmark"]
fn perft_depth_five_benchmark_budget() {
    let game = Game::new();
    let (nodes, elapsed) = timed("perft depth 5", || game.perft(5));

    assert_eq!(nodes, 4_865_609);
    assert_under_budget(
        elapsed,
        budget_ms("CHESS_PERF_PERFT5_MS", 35_000, 7_000),
        "perft(5)",
        "CHESS_PERF_PERFT5_MS",
    );
}

#[test]
#[ignore = "machine-dependent performance benchmark"]
fn search_depth_four_parallel_benchmark_budget() {
    let game = middlegame_position();
    let (result, elapsed) = timed("parallel search depth 4", || {
        game.best_move_with_config(SearchConfig {
            depth: 4,
            parallelism: None,
            ..SearchConfig::default()
        })
    });

    assert!(
        result.best_move.is_some(),
        "search should return a legal move"
    );
    assert!(result.nodes > 0, "search should visit at least one node");
    assert_under_budget(
        elapsed,
        budget_ms("CHESS_PERF_SEARCH4_MS", 12_000, 3_000),
        "depth-4 parallel search",
        "CHESS_PERF_SEARCH4_MS",
    );
}

#[test]
#[ignore = "machine-dependent comparative benchmark"]
fn parallel_search_stays_reasonable_vs_serial() {
    let available_parallelism = thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1);
    if available_parallelism <= 1 {
        println!("skipping comparative parallel search benchmark on a single-core machine");
        return;
    }

    let game = middlegame_position();
    let (serial, serial_elapsed) = timed("serial search depth 4", || {
        game.best_move_with_config(SearchConfig {
            depth: 4,
            parallelism: Some(1),
            ..SearchConfig::default()
        })
    });
    let (parallel, parallel_elapsed) = timed("parallel search depth 4", || {
        game.best_move_with_config(SearchConfig {
            depth: 4,
            parallelism: None,
            ..SearchConfig::default()
        })
    });

    assert_eq!(parallel.best_move, serial.best_move);
    assert_eq!(parallel.score, serial.score);

    let serial_ms = serial_elapsed.as_secs_f64() * 1_000.0;
    let parallel_ms = parallel_elapsed.as_secs_f64() * 1_000.0;
    assert!(
        parallel_ms <= serial_ms * 4.0,
        "parallel search took {:.0}ms vs {:.0}ms serial, which is an unexpectedly large slowdown",
        parallel_ms,
        serial_ms
    );
}
