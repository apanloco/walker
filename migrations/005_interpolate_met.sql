-- Replace the step-function MET lookup with piecewise linear interpolation.
-- Anchor points are midpoints of Compendium of Physical Activities (2024)
-- treadmill-specific ranges (0% grade, normal gait, no load).
-- Between anchors, MET scales linearly with speed.
-- Below 1.61 km/h: flat 2.1 (Compendium says MET 2.1 for the entire <1.0 mph range).
-- Above 8.45 km/h: clamped to 8.3.
--
-- Source: https://pacompendium.com/walking/
-- Conversion: 1 mph = 1.60934 km/h
--
-- Compendium entries used (speeds converted from mph midpoints):
--   17340  MET 2.1  <1.0 mph                        → flat below 1.61 km/h (no interpolation)
--   17343  MET 2.3  1.0 mph           exact    1.00 mph = 1.61 km/h
--   17346  MET 2.8  1.2–1.9 mph       midpoint 1.55 mph = 2.49 km/h
--   17349  MET 3.0  2.0–2.4 mph       midpoint 2.20 mph = 3.54 km/h
--   17352  MET 3.5  2.5–2.9 mph       midpoint 2.70 mph = 4.35 km/h
--   17355  MET 3.8  3.0–3.4 mph       midpoint 3.20 mph = 5.15 km/h
--   17358  MET 4.8  3.5–3.9 mph       midpoint 3.70 mph = 5.95 km/h
--   17361  MET 5.8  4.0–4.4 mph       midpoint 4.20 mph = 6.76 km/h
--   17364  MET 6.8  4.5–4.9 mph       midpoint 4.70 mph = 7.56 km/h
--   17367  MET 8.3  5.0–5.5 mph       midpoint 5.25 mph = 8.45 km/h

CREATE OR REPLACE FUNCTION met_for_speed(speed REAL) RETURNS REAL AS $$
DECLARE
  s REAL[] := ARRAY[1.61, 2.49, 3.54, 4.35, 5.15, 5.95, 6.76, 7.56, 8.45];
  m REAL[] := ARRAY[2.3,  2.8,  3.0,  3.5,  3.8,  4.8,  5.8,  6.8,  8.3];
  i INT;
BEGIN
  -- Flat 2.1 below first anchor (Compendium: <1.0 mph = MET 2.1).
  IF speed < s[1] THEN RETURN 2.1; END IF;
  -- Clamp above highest anchor.
  IF speed >= s[9] THEN RETURN m[9]; END IF;
  -- Find the segment and interpolate.
  FOR i IN 1..8 LOOP
    IF speed <= s[i + 1] THEN
      RETURN m[i] + (speed - s[i]) / (s[i + 1] - s[i]) * (m[i + 1] - m[i]);
    END IF;
  END LOOP;
  RETURN m[9];
END;
$$ LANGUAGE plpgsql IMMUTABLE;
