-- Collapse the calorie-function surface down to one function: active_calories.
--
-- Before this migration the schema carried nine overlapping functions across
-- migrations 002, 005, and 006:
--   met_for_speed(REAL)
--   total_calories_met(REAL, REAL, REAL, REAL)
--   active_calories_met(REAL, REAL, REAL, REAL)
--   total_calories_acsm(REAL, REAL, REAL, REAL)
--   active_calories_acsm(REAL, REAL, REAL, REAL)
--   total_calories(REAL, REAL, REAL, REAL)           — pass-through to _acsm
--   active_calories(REAL, REAL, REAL, REAL)          — pass-through to _acsm
--   total_calories(REAL, REAL, REAL)                 — legacy 3-arg, NULL incline
--   active_calories(REAL, REAL, REAL)                — legacy 3-arg, NULL incline
--
-- That's three separate dimensions of overlap: model variants (MET vs ACSM),
-- pass-through indirection, and 3-arg legacy wrappers. None of the variants
-- besides active_calories(4-arg) had any caller in the application — and
-- total kcal (active + resting) was actively counterproductive on the
-- leaderboard, where it rewarded long sessions over hard ones.
--
-- This migration inlines the ACSM math into a single active_calories(4-arg)
-- and drops the other eight. Swapping to a different walking equation later
-- (e.g. Ludlow–Weyand) is a one-line edit to this function body in a future
-- migration; query sites keep using the generic name.
--
-- Parameter names encode units — Postgres has no unit-aware type system, but
-- the names show up in \df output and tooling, so a misordered call has a
-- visible mismatch with the segment columns it reads from
-- (s.speed_kmh, s.incline_percent, s.weight_kg, s.duration_s).

-- Step 1: redefine active_calories with the ACSM math inlined so it no longer
-- depends on active_calories_acsm. Drop the old signature first because
-- CREATE OR REPLACE cannot change parameter names.
DROP FUNCTION IF EXISTS active_calories(REAL, REAL, REAL, REAL);

CREATE FUNCTION active_calories(
  speed_kmh REAL, incline_percent REAL, weight_kg REAL, duration_s REAL
) RETURNS REAL AS $$
DECLARE
  mmin  REAL := speed_kmh * (1000.0 / 60.0);
  grade REAL := COALESCE(incline_percent, 0.0) / 100.0;
BEGIN
  RETURN (0.1 * mmin + 1.8 * mmin * grade) * weight_kg * duration_s / 12000.0;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Step 2: drop the now-unused overloads and helpers. Use IF EXISTS so the
-- migration is idempotent regardless of which prior state it's applied to.
DROP FUNCTION IF EXISTS total_calories(REAL, REAL, REAL);
DROP FUNCTION IF EXISTS active_calories(REAL, REAL, REAL);
DROP FUNCTION IF EXISTS total_calories(REAL, REAL, REAL, REAL);
DROP FUNCTION IF EXISTS total_calories_acsm(REAL, REAL, REAL, REAL);
DROP FUNCTION IF EXISTS active_calories_acsm(REAL, REAL, REAL, REAL);
DROP FUNCTION IF EXISTS total_calories_met(REAL, REAL, REAL, REAL);
DROP FUNCTION IF EXISTS active_calories_met(REAL, REAL, REAL, REAL);
DROP FUNCTION IF EXISTS met_for_speed(REAL);

-- Step 3: document the surviving public API. Units are encoded in the
-- parameter names; the comment captures only the semantic notes.
COMMENT ON FUNCTION active_calories(REAL, REAL, REAL, REAL) IS
'Active kcal above resting metabolic rate, computed from the ACSM walking equation. NULL incline_percent is treated as 0%.';
