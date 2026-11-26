<?php

use GYDT\SampleResults\ImportHistory;

if (PHP_SAPI !== 'cli') {
    exit(1);
}

require __DIR__ . '/../gydt-sample-results-importer/import-history.php';

$cases = [
    '2025-01-02T07:59:59+00:00' => ['2025-01-01', 1],
    '2025-01-02T08:00:00+00:00' => ['2025-01-02', 86400],
    '2025-07-02T06:59:59+00:00' => ['2025-07-01', 1],
    '2025-07-02T07:00:00+00:00' => ['2025-07-02', 86400],
    '2025-03-09T08:00:00+00:00' => ['2025-03-09', 82800],
    '2025-11-02T07:00:00+00:00' => ['2025-11-02', 90000],
    '2026-11-01T07:00:00+00:00' => ['2026-11-01', 86400],
    '2026-11-23T06:59:59+00:00' => ['2026-11-22', 1],
    '2026-11-23T07:00:00+00:00' => ['2026-11-23', 86400],
];
foreach ($cases as $instant => [$day, $remaining]) {
    $clock = ImportHistory::clock(new DateTimeImmutable($instant));
    if ($clock !== ['day' => $day, 'seconds_to_midnight' => $remaining]) {
        throw new RuntimeException('Unexpected Vancouver history window for ' . $instant);
    }
}

$original = ImportHistory::ownerDay(12, '2025-01-01');
if ($original === ImportHistory::ownerDay(13, '2025-01-01')
    || $original === ImportHistory::ownerDay(12, '2025-01-02')) {
    throw new RuntimeException('History scopes are not distinct by operator and calendar day.');
}

echo "Vancouver calendar rollover, daylight-saving day length and separate operator/day scopes verified.\n";
