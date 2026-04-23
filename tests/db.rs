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
async fn matches_hand_derivation(pool: PgPool) {
    // 4 km/h = 66.667 m/min. At 0% grade, active VO2 = 0.1 * 66.667 = 6.667 ml/kg/min.
    // At 78 kg for 1 h: kcal = 6.667 * 78 * 3600 / 12000 = 156.0.
    let kcal = active_cal(&pool, 4.0, Some(0.0), 78.0, 3600.0).await;
    assert!(
        (kcal - 156.0).abs() < 1.0,
        "active kcal/h at 4.0 km/h, 0%, 78 kg: expected ~156, got {kcal}"
    );

    // 4 km/h at 5% grade adds 1.8 * 66.667 * 0.05 = 6.0 ml/kg/min of grade term.
    // Total active VO2 = 6.667 + 6.0 = 12.667. kcal = 12.667 * 78 * 3600 / 12000 = 296.4.
    let kcal = active_cal(&pool, 4.0, Some(5.0), 78.0, 3600.0).await;
    assert!(
        (kcal - 296.4).abs() < 1.0,
        "active kcal/h at 4.0 km/h, 5%, 78 kg: expected ~296, got {kcal}"
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
