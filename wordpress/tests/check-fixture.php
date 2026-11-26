<?php

use GYDT\SampleResults\Archive;
use GYDT\SampleResults\Package;
use GYDT\SampleResults\Rejection;

require __DIR__ . '/../gydt-sample-results-importer/contract.php';
require __DIR__ . '/../gydt-sample-results-importer/zip.php';

if (PHP_SAPI !== 'cli') {
    exit(1);
}
if ($argc !== 2) {
    fwrite(STDERR, "Usage: php check-fixture.php /path/to/QA-SYNTHETIC-ZIP-CONTRACT.zip\n");
    exit(2);
}

$sample = Package::load($argv[1], Archive::MAX_ZIP);
if ($sample->fingerprint !== 'd274aec7710363c27ead5e96da5341df9b972ccb4bf013d69b86f5983cff46bf'
    || array_column($sample->data['results'], 'percentage') !== ['12-18%', 'blank']
    || $sample->collectionDate->format('Y-m-d H:i:s P') !== '2026-11-19 00:00:00 -07:00'
    || $sample->collectionDate->setTimezone(new DateTimeZone('UTC'))->format('Y-m-d H:i:s') !== '2026-11-19 07:00:00'
    || hash('sha256', $sample->photo) !== '72eec649b5181e2ae62deee68b0b4e747b99c1028d947943e035a35e1848be75') {
    throw new RuntimeException('The task 1 fixture did not match the validated contract.');
}
try {
    Package::load($argv[1], filesize($argv[1]) - 1);
    throw new RuntimeException('The lower upload limit did not reject the fixture.');
} catch (Rejection) {
    echo "Task 1 fixture, canonical fingerprint, result order, JPEG and lower limit verified.\n";
}

$archive = file_get_contents($argv[1]);
$temporary = tempnam(sys_get_temp_dir(), 'gydt-fixture-');
if ($archive === false || substr($archive, -2) !== "\0\0" || $temporary === false) {
    throw new RuntimeException('Could not prepare a copy of the uncommented fixture.');
}
try {
    foreach (['Synthetic archive comment', str_repeat('a', 65535)] as $comment) {
        file_put_contents($temporary, substr($archive, 0, -2) . pack('v', strlen($comment)) . $comment);
        $commented = Package::load($temporary, Archive::MAX_ZIP);
        if ($commented->fingerprint !== $sample->fingerprint || $commented->archiveHash === $sample->archiveHash) {
            throw new RuntimeException('An archive comment should affect only the hash of the ZIP bytes.');
        }
    }

    file_put_contents($temporary, substr($archive, 0, -2) . pack('v', 2) . 'a');
    try {
        Package::load($temporary, Archive::MAX_ZIP);
        throw new RuntimeException('An incorrect archive comment length was accepted.');
    } catch (Rejection) {
        echo "Ordinary and maximum-length archive comments accepted. Incorrect length rejected.\n";
    }
} finally {
    unlink($temporary);
}
