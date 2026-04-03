-- Step 2: drop the calories_kcal column.
-- All calorie values are now computed at query time via total_calories() and active_calories().
ALTER TABLE segments DROP COLUMN calories_kcal;
