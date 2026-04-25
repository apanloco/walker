use sqlx::{PgPool, Row};

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

async fn create_user(pool: &PgPool) -> uuid::Uuid {
    sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO users (email, display_name) VALUES ($1, 'Test') RETURNING id",
    )
    .bind(format!("test-{}@test.local", uuid::Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
async fn insert_strava_segment(
    pool: &PgPool,
    user_id: uuid::Uuid,
    started_at: &str,
    speed_kmh: f32,
    duration_s: f32,
    distance_m: f32,
    weight_kg: f32,
    external_id: &str,
) -> u64 {
    let act_row = sqlx::query(
        "INSERT INTO imported_activities (source, external_id, raw_data)
         VALUES ('strava', $1, '{}')
         ON CONFLICT (source, external_id) DO UPDATE SET raw_data = imported_activities.raw_data
         RETURNING id",
    )
    .bind(external_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let activity_id: i64 = act_row.get("id");

    sqlx::query(
        "INSERT INTO segments
             (user_id, started_at, moving, speed_kmh, duration_s, distance_m, weight_kg,
              open, source, activity_id, last_heartbeat_at)
         VALUES ($1, $2::timestamptz, true, $3, $4, $5, $6, false, 'strava', $7, NOW())
         ON CONFLICT (user_id, activity_id) WHERE activity_id IS NOT NULL DO NOTHING",
    )
    .bind(user_id)
    .bind(started_at)
    .bind(speed_kmh)
    .bind(duration_s)
    .bind(distance_m)
    .bind(weight_kg)
    .bind(activity_id)
    .execute(pool)
    .await
    .unwrap()
    .rows_affected()
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

// -- Strava import: segment insert and calorie round-trip --

#[sqlx::test(migrations = "./migrations")]
async fn strava_segment_insert_and_query(pool: PgPool) {
    let user_id = create_user(&pool).await;

    let rows = insert_strava_segment(
        &pool,
        user_id,
        "2026-01-15T10:00:00Z",
        5.15,
        3600.0,
        5150.0,
        70.0,
        "strava-act-001",
    )
    .await;
    assert_eq!(rows, 1, "First insert should affect 1 row");

    // Ludlow-Weyand at 5.15 km/h, NULL incline (0%), 70 kg:
    // s = 5.15/3.6 = 1.431 m/s. vo2 = 3.28 + 2.66 * 1.431^2 = 8.72 ml/kg/min.
    // kcal = 8.72 * 70 * 3600 / 12000 ≈ 183.
    let active: f32 = sqlx::query_scalar(
        "SELECT active_calories(s.speed_kmh, NULL, s.weight_kg, s.duration_s)
         FROM segments s
         JOIN imported_activities ia ON ia.id = s.activity_id
         WHERE s.user_id = $1 AND ia.external_id = $2",
    )
    .bind(user_id)
    .bind("strava-act-001")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(
        (active - 183.0).abs() < 1.0,
        "Active kcal for Strava walk segment: expected ~183, got {active}"
    );

    let row = sqlx::query(
        "SELECT s.source, s.open FROM segments s
         JOIN imported_activities ia ON ia.id = s.activity_id
         WHERE s.user_id = $1 AND ia.external_id = $2",
    )
    .bind(user_id)
    .bind("strava-act-001")
    .fetch_one(&pool)
    .await
    .unwrap();

    let source: String = row.get("source");
    let open: bool = row.get("open");
    assert_eq!(source, "strava");
    assert!(!open, "Imported segment must be closed");
}

#[sqlx::test(migrations = "./migrations")]
async fn strava_segment_deduplication(pool: PgPool) {
    let user_id = create_user(&pool).await;

    let first = insert_strava_segment(
        &pool,
        user_id,
        "2026-01-15T10:00:00Z",
        5.0,
        1800.0,
        2500.0,
        70.0,
        "strava-act-dup",
    )
    .await;

    let second = insert_strava_segment(
        &pool,
        user_id,
        "2026-01-15T10:00:00Z",
        5.0,
        1800.0,
        2500.0,
        70.0,
        "strava-act-dup",
    )
    .await;

    assert_eq!(first, 1, "First insert should succeed");
    assert_eq!(
        second, 0,
        "Duplicate external_id should be silently ignored"
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM segments s
         JOIN imported_activities ia ON ia.id = s.activity_id
         WHERE s.user_id = $1 AND ia.external_id = $2",
    )
    .bind(user_id)
    .bind("strava-act-dup")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        count, 1,
        "Exactly one segment should exist after duplicate insert"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn strava_latest_segment_unix(pool: PgPool) {
    let user_id = create_user(&pool).await;

    let ts: Option<i64> = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM MAX(started_at))::BIGINT
         FROM segments WHERE user_id = $1 AND source = 'strava'",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(ts.is_none(), "Expected None with no Strava segments");

    insert_strava_segment(
        &pool,
        user_id,
        "2026-01-10T08:00:00Z",
        5.0,
        1800.0,
        2500.0,
        70.0,
        "strava-old",
    )
    .await;
    insert_strava_segment(
        &pool,
        user_id,
        "2026-01-15T10:00:00Z",
        6.0,
        3600.0,
        6000.0,
        70.0,
        "strava-new",
    )
    .await;

    let ts: Option<i64> = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM MAX(started_at))::BIGINT
         FROM segments WHERE user_id = $1 AND source = 'strava'",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 2026-01-15T10:00:00Z = 1768471200 Unix
    let ts = ts.expect("Expected a timestamp after inserting segments");
    assert!(
        (ts - 1768471200).abs() < 2,
        "Expected Unix ts ~1768471200 (2026-01-15T10:00:00Z), got {ts}"
    );
}
