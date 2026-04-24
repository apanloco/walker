-- Upgrade active_calories to Ludlow-Weyand (2017) for walking and Minetti (2002) for running.
-- Dispatch is speed-based: >= 7 km/h → Minetti (running gait), else → Ludlow-Weyand (walking gait).
-- The 7 km/h threshold matches the physiological gait transition and applies uniformly
-- to BLE and Strava data — no sport_type or source column needed.
--
-- Walk / BLE (< 7 km/h) → Ludlow-Weyand minimum mechanics:
--   Active VO2 (ml/kg/min):
--     grade >= 0: 0.32g + 3.28 + (1 + 0.19g) * 2.66 * s^2
--     grade <  0: 0.73 * (3.28 + 2.66 * s^2)
--   where g = incline_percent (percent grade, may be negative), s = speed_kmh / 3.6 (m/s)
--   kcal = active_vo2 * weight_kg * duration_s / 12000
--
-- Run (>= 7 km/h) → Minetti (2002) 5th-order polynomial:
--   Cr(g) = 155.4g^5 - 30.4g^4 - 43.3g^3 + 46.3g^2 + 19.5g + 3.6  (J/kg/m, net above resting)
--   where g = incline_percent / 100 (decimal, valid range -0.45 to +0.45)
--   kcal = Cr * weight_kg * (speed_kmh / 3.6 * duration_s) / 4184
--
-- Sources:
--   Ludlow & Weyand (2017) J Appl Physiol 123:1288-1302
--   Minetti et al. (2002) J Appl Physiol 93:1039-1046

DROP FUNCTION IF EXISTS active_calories(REAL, REAL, REAL, REAL);

CREATE FUNCTION active_calories(
  speed_kmh REAL, incline_percent REAL, weight_kg REAL, duration_s REAL
) RETURNS REAL AS $$
DECLARE
  s   REAL := speed_kmh / 3.6;
  g   REAL := COALESCE(incline_percent, 0.0);
  vo2 REAL;
  cr  REAL;
BEGIN
  IF speed_kmh >= 7.0 THEN
    -- Minetti (2002): net cost of running as a function of grade.
    g := g / 100.0;
    cr := 155.4 * g^5 - 30.4 * g^4 - 43.3 * g^3 + 46.3 * g^2 + 19.5 * g + 3.6;
    RETURN cr * weight_kg * (s * duration_s) / 4184.0;
  ELSE
    -- Ludlow-Weyand (2017): minimum mechanics walking model.
    IF g >= 0.0 THEN
      vo2 := 0.32 * g + 3.28 + (1.0 + 0.19 * g) * 2.66 * s * s;
    ELSE
      vo2 := 0.73 * (3.28 + 2.66 * s * s);
    END IF;
    RETURN vo2 * weight_kg * duration_s / 12000.0;
  END IF;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

COMMENT ON FUNCTION active_calories(REAL, REAL, REAL, REAL) IS
'Active kcal above resting metabolic rate. Speed-dispatched: Ludlow-Weyand (2017) for < 7 km/h, Minetti (2002) for >= 7 km/h. NULL incline_percent treated as 0%.';
