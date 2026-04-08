use sqlx::PgPool;

/// Helper: query met_for_speed() from the database.
async fn met(pool: &PgPool, speed: f32) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT met_for_speed($1)")
        .bind(speed)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Helper: query total_calories() from the database.
async fn total_cal(pool: &PgPool, speed: f32, weight: f32, duration: f32) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT total_calories($1, $2, $3)")
        .bind(speed)
        .bind(weight)
        .bind(duration)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Helper: query active_calories() from the database.
async fn active_cal(pool: &PgPool, speed: f32, weight: f32, duration: f32) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT active_calories($1, $2, $3)")
        .bind(speed)
        .bind(weight)
        .bind(duration)
        .fetch_one(pool)
        .await
        .unwrap()
}

// -- MET anchor points --
// Each anchor is at the midpoint of a Compendium range (or exact speed for single-point entries).
// See migrations/005_interpolate_met.sql for the full source table.

#[sqlx::test(migrations = "./migrations")]
async fn met_at_anchors(pool: PgPool) {
    let cases = [
        (0.80, 2.1),
        (1.61, 2.3),
        (2.49, 2.8),
        (3.54, 3.0),
        (4.35, 3.5),
        (5.15, 3.8),
        (5.95, 4.8),
        (6.76, 5.8),
        (7.56, 6.8),
        (8.45, 8.3),
    ];
    for (speed, expected_met) in cases {
        let result = met(&pool, speed).await;
        assert!(
            (result - expected_met).abs() < 0.05,
            "MET at {speed} km/h: expected {expected_met}, got {result}"
        );
    }
}

// -- Interpolation between anchors --

#[sqlx::test(migrations = "./migrations")]
async fn met_interpolates_between_anchors(pool: PgPool) {
    // Midpoint between anchor 0.80 (MET 2.1) and anchor 1.61 (MET 2.3).
    // Expected: 2.1 + 0.5 * (2.3 - 2.1) = 2.2
    let result = met(&pool, 1.205).await;
    assert!(
        (result - 2.2).abs() < 0.05,
        "MET at 1.205 km/h: expected ~2.2, got {result}"
    );

    // Midpoint between anchor 1.61 (MET 2.3) and anchor 2.49 (MET 2.8).
    // Expected: 2.3 + 0.5 * (2.8 - 2.3) = 2.55
    let result = met(&pool, 2.05).await;
    assert!(
        (result - 2.55).abs() < 0.05,
        "MET at 2.05 km/h: expected ~2.55, got {result}"
    );

    // 6.0 km/h — between anchor 5.95 (MET 4.8) and 6.76 (MET 5.8).
    // Expected: 4.8 + (6.0 - 5.95) / (6.76 - 5.95) * (5.8 - 4.8) = 4.862
    let result = met(&pool, 6.0).await;
    assert!(
        (result - 4.86).abs() < 0.1,
        "MET at 6.0 km/h: expected ~4.86, got {result}"
    );
}

// -- Monotonicity --

#[sqlx::test(migrations = "./migrations")]
async fn met_is_monotonically_increasing(pool: PgPool) {
    let mut prev_met = 0.0_f32;
    let mut speed = 0.5_f32;
    while speed <= 10.0 {
        let m = met(&pool, speed).await;
        assert!(
            m >= prev_met,
            "MET decreased: {prev_met} at previous speed -> {m} at {speed} km/h"
        );
        prev_met = m;
        speed += 0.1;
    }
}

// -- Edge cases --

#[sqlx::test(migrations = "./migrations")]
async fn met_clamps_at_extremes(pool: PgPool) {
    let result = met(&pool, 0.0).await;
    assert!(
        (result - 2.1).abs() < 0.05,
        "MET at 0 km/h: expected 2.1, got {result}"
    );

    let result = met(&pool, 15.0).await;
    assert!(
        (result - 8.3).abs() < 0.05,
        "MET at 15 km/h: expected 8.3, got {result}"
    );
}

// -- Calorie formulas --

#[sqlx::test(migrations = "./migrations")]
async fn calorie_formulas_correct(pool: PgPool) {
    // 1 hour at 2.49 km/h (anchor, MET 2.8), 70 kg.
    // Total: 2.8 * 70 * 3600 / 3600 = 196 kcal
    // Active: (2.8 - 1) * 70 * 3600 / 3600 = 126 kcal
    let total = total_cal(&pool, 2.49, 70.0, 3600.0).await;
    assert!(
        (total - 196.0).abs() < 1.0,
        "Total cal: expected ~196, got {total}"
    );

    let active = active_cal(&pool, 2.49, 70.0, 3600.0).await;
    assert!(
        (active - 126.0).abs() < 1.0,
        "Active cal: expected ~126, got {active}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn active_calories_less_than_total(pool: PgPool) {
    let speeds = [1.0, 2.5, 3.5, 5.0, 7.0, 9.0];
    for speed in speeds {
        let total = total_cal(&pool, speed, 70.0, 3600.0).await;
        let active = active_cal(&pool, speed, 70.0, 3600.0).await;
        assert!(
            active < total,
            "Active ({active}) should be less than total ({total}) at {speed} km/h"
        );
        assert!(active > 0.0, "Active calories should be positive at {speed} km/h");
    }
}
