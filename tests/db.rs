use sqlx::PgPool;

/// Helper: query met_for_speed() from the database.
async fn met(pool: &PgPool, speed: f32) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT met_for_speed($1)")
        .bind(speed)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Helper: query total_calories() from the database. Passes NULL incline
/// (the default for devices without an incline sensor, equivalent to 0%).
async fn total_cal(pool: &PgPool, speed: f32, weight: f32, duration: f32) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT total_calories($1, NULL, $2, $3)")
        .bind(speed)
        .bind(weight)
        .bind(duration)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Helper: query active_calories() from the database. Passes NULL incline.
async fn active_cal(pool: &PgPool, speed: f32, weight: f32, duration: f32) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT active_calories($1, NULL, $2, $3)")
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
    // At exact anchor speeds, MET should match the Compendium value within
    // float rounding (tight tolerance — these are exact lookups, not interpolation).
    let cases = [
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
            (result - expected_met).abs() < 0.001,
            "MET at {speed} km/h: expected {expected_met}, got {result}"
        );
    }
}

// -- Flat region below first anchor --

#[sqlx::test(migrations = "./migrations")]
async fn met_flat_below_first_anchor(pool: PgPool) {
    // Compendium says MET 2.1 for the entire <1.0 mph range.
    // No interpolation — should be exactly 2.1 everywhere below 1.61 km/h.
    for speed in [0.0, 0.5, 0.8, 1.0, 1.2, 1.5, 1.6] {
        let result = met(&pool, speed).await;
        assert!(
            (result - 2.1).abs() < 0.001,
            "MET at {speed} km/h: expected flat 2.1, got {result}"
        );
    }
}

// -- Interpolation between anchors --

#[sqlx::test(migrations = "./migrations")]
async fn met_interpolates_between_anchors(pool: PgPool) {
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
    // MET should never decrease as speed increases.
    // Start at 1.61 (first anchor) — below that is flat 2.1.
    let mut prev_met = 2.1_f32;
    let mut speed = 1.61_f32;
    while speed <= 10.0 {
        let m = met(&pool, speed).await;
        assert!(
            m >= prev_met - 0.001, // float tolerance
            "MET decreased: {prev_met} at previous speed -> {m} at {speed} km/h"
        );
        prev_met = m;
        speed += 0.1;
    }
}

// -- Edge cases --

#[sqlx::test(migrations = "./migrations")]
async fn met_clamps_at_extremes(pool: PgPool) {
    // Very fast — should clamp to highest MET (8.3).
    let result = met(&pool, 15.0).await;
    assert!(
        (result - 8.3).abs() < 0.001,
        "MET at 15 km/h: expected 8.3, got {result}"
    );
}

// -- Step at 1.61 km/h boundary --

#[sqlx::test(migrations = "./migrations")]
async fn met_step_at_boundary(pool: PgPool) {
    // Just below 1.61: flat 2.1
    let below = met(&pool, 1.60).await;
    assert!(
        (below - 2.1).abs() < 0.001,
        "MET at 1.60 km/h: expected 2.1, got {below}"
    );

    // At 1.61: anchor 2.3
    let at = met(&pool, 1.61).await;
    assert!(
        (at - 2.3).abs() < 0.001,
        "MET at 1.61 km/h: expected 2.3, got {at}"
    );
}

// -- MET model (direct, not via the active wrapper which now points at ACSM) --

async fn total_cal_met(pool: &PgPool, speed: f32, weight: f32, duration: f32) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT total_calories_met($1, NULL, $2, $3)")
        .bind(speed)
        .bind(weight)
        .bind(duration)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn active_cal_met(pool: &PgPool, speed: f32, weight: f32, duration: f32) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT active_calories_met($1, NULL, $2, $3)")
        .bind(speed)
        .bind(weight)
        .bind(duration)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn met_calorie_formulas_correct(pool: PgPool) {
    // 1 hour at 2.49 km/h (anchor, MET 2.8), 70 kg.
    // Total: 2.8 * 70 * 3600 / 3600 = 196 kcal
    // Active: (2.8 - 1) * 70 * 3600 / 3600 = 126 kcal
    let total = total_cal_met(&pool, 2.49, 70.0, 3600.0).await;
    assert!(
        (total - 196.0).abs() < 1.0,
        "MET total cal: expected ~196, got {total}"
    );

    let active = active_cal_met(&pool, 2.49, 70.0, 3600.0).await;
    assert!(
        (active - 126.0).abs() < 1.0,
        "MET active cal: expected ~126, got {active}"
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
        assert!(
            active > 0.0,
            "Active calories should be positive at {speed} km/h"
        );
    }
}

// -- ACSM model (direct, bypassing the env-var wrapper) --

async fn active_acsm(pool: &PgPool, speed: f32, incline: f32, weight: f32, duration: f32) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT active_calories_acsm($1, $2, $3, $4)")
        .bind(speed)
        .bind(incline)
        .bind(weight)
        .bind(duration)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn acsm_matches_hand_derivation(pool: PgPool) {
    // 4 km/h = 66.667 m/min. At 0% grade, active VO2 = 0.1 * 66.667 = 6.667 ml/kg/min.
    // At 78 kg for 1 h: kcal = 6.667 * 78 * 3600 / 12000 = 156.0.
    let kcal = active_acsm(&pool, 4.0, 0.0, 78.0, 3600.0).await;
    assert!(
        (kcal - 156.0).abs() < 1.0,
        "ACSM active kcal/h at 4.0 km/h, 0%, 78 kg: expected ~156, got {kcal}"
    );

    // 4 km/h at 5% grade adds 1.8 * 66.667 * 0.05 = 6.0 ml/kg/min of grade term.
    // Total active VO2 = 6.667 + 6.0 = 12.667. kcal = 12.667 * 78 * 3600 / 12000 = 296.4.
    let kcal = active_acsm(&pool, 4.0, 5.0, 78.0, 3600.0).await;
    assert!(
        (kcal - 296.4).abs() < 1.0,
        "ACSM active kcal/h at 4.0 km/h, 5%, 78 kg: expected ~296, got {kcal}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn acsm_null_incline_equals_zero_incline(pool: PgPool) {
    // Devices without an incline sensor send NULL. Calorie output must equal
    // explicit 0% — this is the promise behind the NULL semantic.
    let null_kcal: f32 = sqlx::query_scalar("SELECT active_calories_acsm($1, NULL, $2, $3)")
        .bind(4.0_f32)
        .bind(78.0_f32)
        .bind(3600.0_f32)
        .fetch_one(&pool)
        .await
        .unwrap();
    let zero_kcal = active_acsm(&pool, 4.0, 0.0, 78.0, 3600.0).await;
    assert!(
        (null_kcal - zero_kcal).abs() < 0.01,
        "NULL incline ({null_kcal}) must equal 0% incline ({zero_kcal})"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn acsm_grows_with_incline(pool: PgPool) {
    // At any nonzero speed, kcal must strictly increase with incline.
    let level = active_acsm(&pool, 4.0, 0.0, 78.0, 3600.0).await;
    let five = active_acsm(&pool, 4.0, 5.0, 78.0, 3600.0).await;
    let ten = active_acsm(&pool, 4.0, 10.0, 78.0, 3600.0).await;
    assert!(level < five, "0% ({level}) should be less than 5% ({five})");
    assert!(five < ten, "5% ({five}) should be less than 10% ({ten})");
}
