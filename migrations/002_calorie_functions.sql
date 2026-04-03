-- MET lookup function (Compendium 2024, treadmill-specific).
-- Single source of truth for the MET table — no duplication in Rust or JS.
CREATE OR REPLACE FUNCTION met_for_speed(speed REAL) RETURNS REAL AS $$
BEGIN
  IF speed < 1.6 THEN RETURN 2.1;
  ELSIF speed <= 3.0 THEN RETURN 2.8;
  ELSIF speed <= 3.9 THEN RETURN 3.0;
  ELSIF speed <= 4.7 THEN RETURN 3.5;
  ELSIF speed <= 5.5 THEN RETURN 3.8;
  ELSIF speed <= 6.3 THEN RETURN 4.8;
  ELSIF speed <= 7.1 THEN RETURN 5.8;
  ELSIF speed <= 7.9 THEN RETURN 6.8;
  ELSE RETURN 8.3;
  END IF;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Total calories: MET × weight × duration / 3600 (includes resting metabolic rate).
CREATE OR REPLACE FUNCTION total_calories(speed REAL, weight REAL, duration REAL) RETURNS REAL AS $$
BEGIN
  RETURN met_for_speed(speed) * weight * duration / 3600.0;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Active calories: (MET - 1) × weight × duration / 3600 (exercise-only, excludes resting).
CREATE OR REPLACE FUNCTION active_calories(speed REAL, weight REAL, duration REAL) RETURNS REAL AS $$
BEGIN
  RETURN (met_for_speed(speed) - 1.0) * weight * duration / 3600.0;
END;
$$ LANGUAGE plpgsql IMMUTABLE;
