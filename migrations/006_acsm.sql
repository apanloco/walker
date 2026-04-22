-- Switch the active calorie model to ACSM (incline-aware), with MET kept as
-- a reference implementation for comparison (`walker compare-calorie-models`).
--
--   acsm — ACSM walking equation, incline-aware:
--          VO2 (ml/kg/min) = 0.1 * speed_m_per_min + 1.8 * speed_m_per_min * grade + 3.5
--            where grade is a decimal fraction (5% → 0.05).
--          kcal = VO2 * weight_kg * duration_s / 12000
--            (ml/kg/min × kg = ml/min; /1000 = L/min; ×5 kcal/L; ×duration_s/60 = kcal)
--   met  — Compendium MET lookup (from migration 005), incline ignored.
--
-- Active calories subtract the resting 3.5 ml/kg/min term. For MET this is
-- (MET − 1); for ACSM it is the full speed + grade VO2 sum without the +3.5.
--
-- `active_calories` / `total_calories` are thin pass-through wrappers that
-- currently delegate to `_acsm`. Keeping them as the abstraction point means
-- adding a new model later (e.g. Ludlow–Weyand) is a one-line change to the
-- wrapper body — query sites never need to know which model is in use.
--
-- Source: ACSM Guidelines for Exercise Testing and Prescription (walking
-- equation); https://pacompendium.com/walking/ (MET).

-- Incline per segment. NULL = device doesn't report incline; calorie functions
-- COALESCE to 0%, so historical segments behave identically to today.
ALTER TABLE segments ADD COLUMN incline_percent REAL;

-- ACSM: VO2 × weight × duration / 12000. Active omits the +3.5 resting term.
CREATE OR REPLACE FUNCTION total_calories_acsm(
  speed REAL, incline REAL, weight REAL, duration REAL
) RETURNS REAL AS $$
DECLARE
  mmin  REAL := speed * (1000.0 / 60.0);
  grade REAL := COALESCE(incline, 0.0) / 100.0;
BEGIN
  RETURN (0.1 * mmin + 1.8 * mmin * grade + 3.5) * weight * duration / 12000.0;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE OR REPLACE FUNCTION active_calories_acsm(
  speed REAL, incline REAL, weight REAL, duration REAL
) RETURNS REAL AS $$
DECLARE
  mmin  REAL := speed * (1000.0 / 60.0);
  grade REAL := COALESCE(incline, 0.0) / 100.0;
BEGIN
  RETURN (0.1 * mmin + 1.8 * mmin * grade) * weight * duration / 12000.0;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- MET: 4-arg signature parallels ACSM. Incline is accepted but ignored.
CREATE OR REPLACE FUNCTION total_calories_met(
  speed REAL, incline REAL, weight REAL, duration REAL
) RETURNS REAL AS $$
BEGIN
  RETURN met_for_speed(speed) * weight * duration / 3600.0;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE OR REPLACE FUNCTION active_calories_met(
  speed REAL, incline REAL, weight REAL, duration REAL
) RETURNS REAL AS $$
BEGIN
  RETURN (met_for_speed(speed) - 1.0) * weight * duration / 3600.0;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Add 4-arg wrappers pointing at ACSM, then repoint the legacy 3-arg wrappers
-- at the same incline-aware path. This keeps deploys and rollbacks compatible
-- without carrying a follow-up migration for an unreleased schema change.

CREATE OR REPLACE FUNCTION total_calories(
  speed REAL, incline REAL, weight REAL, duration REAL
) RETURNS REAL AS $$
BEGIN
  RETURN total_calories_acsm(speed, incline, weight, duration);
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE OR REPLACE FUNCTION active_calories(
  speed REAL, incline REAL, weight REAL, duration REAL
) RETURNS REAL AS $$
BEGIN
  RETURN active_calories_acsm(speed, incline, weight, duration);
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE OR REPLACE FUNCTION total_calories(
  speed REAL, weight REAL, duration REAL
) RETURNS REAL AS $$
BEGIN
  RETURN total_calories(speed, NULL, weight, duration);
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE OR REPLACE FUNCTION active_calories(
  speed REAL, weight REAL, duration REAL
) RETURNS REAL AS $$
BEGIN
  RETURN active_calories(speed, NULL, weight, duration);
END;
$$ LANGUAGE plpgsql IMMUTABLE;
