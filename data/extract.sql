-- Lower Manhattan road segments for overrules-city.
-- Overture release 2026-09-23.1 (schema 2.0.0). Run from the repository root: `just data`.
INSTALL httpfs; LOAD httpfs;
SET s3_region='us-west-2';
SET enable_progress_bar=false;

COPY (
  SELECT * FROM read_parquet('s3://overturemaps-us-west-2/release/2026-09-23.1/theme=transportation/type=segment/*.parquet', hive_partitioning=false)
  WHERE subtype = 'road'
    AND bbox.xmin >= -74.0200 AND bbox.xmax <= -73.9700
    AND bbox.ymin >= 40.7000 AND bbox.ymax <= 40.7350
) TO 'data/segment.parquet' (FORMAT PARQUET, COMPRESSION ZSTD);

SELECT count(*) AS segments FROM 'data/segment.parquet';
