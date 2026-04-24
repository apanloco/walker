use sqlx::PgPool;

async fn active_cal(
    pool: &PgPool,
    speed: f32,
    incline: Option<f32>,
    weight: f32,
    duration: f32,
) -> f32 {
    sqlx::query_scalar::<_, f32>("SELECT active_calories($1, $2, $3, $4)")
        .bind(speed)
        .bind(incline)
        .bind(weight)
        .bind(duration)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn matches_hand_derivation_ludlow_weyand(pool: PgPool) {
    // Ludlow-Weyand, 4 km/h, 0% grade: s = 4/3.6 = 1.111 m/s, g = 0.
    // active_vo2 = 3.28 + 2.66 * 1.111^2 = 3.28 + 3.284 = 6.564 ml/kg/min.
    // kcal = 6.564 * 78 * 3600 / 12000 = 153.6.
    let kcal = active_cal(&pool, 4.0, Some(0.0), 78.0, 3600.0).await;
    assert!(
        (kcal - 153.6).abs() < 1.0,
        "active kcal/h at 4.0 km/h, 0%, 78 kg: expected ~153.6, got {kcal}"
    );

    // Ludlow-Weyand, 4 km/h, 5% grade: g = 5.
    // active_vo2 = 0.32*5 + 3.28 + (1 + 0.19*5) * 2.66 * 1.111^2
    //            = 1.6 + 3.28 + 1.95 * 3.284 = 11.284 ml/kg/min.
    // kcal = 11.284 * 78 * 3600 / 12000 = 264.0.
    let kcal = active_cal(&pool, 4.0, Some(5.0), 78.0, 3600.0).await;
    assert!(
        (kcal - 264.0).abs() < 1.0,
        "active kcal/h at 4.0 km/h, 5%, 78 kg: expected ~264, got {kcal}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn matches_hand_derivation_minetti(pool: PgPool) {
    // Minetti (2002), 10 km/h >= 7 km/h threshold so running model is used.
    // g = 0.0, Cr(0) = 3.6 J/kg/m. dist = (10/3.6) * 3600 = 10000 m.
    // kcal = 3.6 * 70 * 10000 / 4184 = 602.3.
    let kcal = active_cal(&pool, 10.0, Some(0.0), 70.0, 3600.0).await;
    assert!(
        (kcal - 602.3).abs() < 1.0,
        "active kcal/h at 10.0 km/h, 0%, 70 kg: expected ~602.3, got {kcal}"
    );

    // g = 0.05. Cr(0.05) = 155.4*(0.05^5) - 30.4*(0.05^4) - 43.3*(0.05^3)
    //                      + 46.3*(0.05^2) + 19.5*0.05 + 3.6 ≈ 4.685 J/kg/m.
    // kcal = 4.685 * 70 * 10000 / 4184 = 783.9.
    let kcal = active_cal(&pool, 10.0, Some(5.0), 70.0, 3600.0).await;
    assert!(
        (kcal - 783.9).abs() < 1.0,
        "active kcal/h at 10.0 km/h, 5%, 70 kg: expected ~783.9, got {kcal}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn downhill_ludlow_weyand(pool: PgPool) {
    // Downhill (g < 0) uses the flat branch: 0.73 * (3.28 + 2.66 * s^2).
    // 3.0 km/h, -5%: s = 3/3.6 = 0.8333 m/s.
    // vo2 = 0.73 * (3.28 + 2.66 * 0.8333^2) = 0.73 * 5.127 = 3.743 ml/kg/min.
    // kcal = 3.743 * 70 * 3600 / 12000 = 78.6.
    let kcal = active_cal(&pool, 3.0, Some(-5.0), 70.0, 3600.0).await;
    assert!(
        (kcal - 78.6).abs() < 1.0,
        "active kcal/h at 3.0 km/h, -5%, 70 kg: expected ~78.6, got {kcal}"
    );
    // Grade doesn't affect downhill — only sign matters.
    let kcal2 = active_cal(&pool, 3.0, Some(-10.0), 70.0, 3600.0).await;
    assert!(
        (kcal - kcal2).abs() < 0.01,
        "downhill at -5% ({kcal}) should equal -10% ({kcal2})"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn null_incline_equals_zero_incline(pool: PgPool) {
    // Devices without an incline sensor send NULL. Calorie output must equal
    // explicit 0% — this is the promise behind the NULL semantic.
    let null_kcal = active_cal(&pool, 4.0, None, 78.0, 3600.0).await;
    let zero_kcal = active_cal(&pool, 4.0, Some(0.0), 78.0, 3600.0).await;
    assert!(
        (null_kcal - zero_kcal).abs() < 0.01,
        "NULL incline ({null_kcal}) must equal 0% incline ({zero_kcal})"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn grows_with_incline(pool: PgPool) {
    // At any nonzero speed, kcal must strictly increase with incline.
    let level = active_cal(&pool, 4.0, Some(0.0), 78.0, 3600.0).await;
    let five = active_cal(&pool, 4.0, Some(5.0), 78.0, 3600.0).await;
    let ten = active_cal(&pool, 4.0, Some(10.0), 78.0, 3600.0).await;
    assert!(level < five, "0% ({level}) should be less than 5% ({five})");
    assert!(five < ten, "5% ({five}) should be less than 10% ({ten})");
}

#[sqlx::test(migrations = "./migrations")]
async fn positive_across_walking_speeds(pool: PgPool) {
    let speeds = [1.0, 2.5, 3.5, 5.0, 7.0, 9.0];
    for speed in speeds {
        let active = active_cal(&pool, speed, None, 70.0, 3600.0).await;
        assert!(
            active > 0.0,
            "Active calories should be positive at {speed} km/h, got {active}"
        );
    }
}
