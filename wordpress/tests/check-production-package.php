<?php

use GYDT\SampleResults\Archive;
use GYDT\SampleResults\Package;
use GYDT\SampleResults\Rejection;

if (PHP_SAPI !== 'cli' || $argc !== 3) {
    exit(1);
}

$plugin = $argv[1];
require $plugin . '/contract.php';
require $plugin . '/zip.php';

foreach (['synthetic-legacy-jpg.zip' => false, 'synthetic-opus-and-jpg.zip' => true] as $name => $opus) {
    $path = $argv[2] . '/' . $name;
    $archive = new Archive($path, Archive::MAX_ZIP);
    $package = Package::load($path, Archive::MAX_ZIP);
    $jpeg = array_values(array_filter($archive->names(), static fn (string $file): bool => in_array(strtolower(pathinfo($file, PATHINFO_EXTENSION)), ['jpg', 'jpeg'], true)));
    if (count($jpeg) !== 1 || $package->photo !== $archive->read($jpeg[0])) {
        throw new RuntimeException('The production importer did not preserve the JPEG bytes.');
    }
    if ($opus) {
        $member = $package->data['submission_number'] . '.0';
        if ($package->spectrumName !== $member || $package->spectrum !== $archive->read($member)
            || $package->spectrumHash !== hash('sha256', $package->spectrum)) {
            throw new RuntimeException('The production importer did not preserve the exact OPUS name and bytes.');
        }
    } elseif ($package->spectrum !== null) {
        throw new RuntimeException('An ordinary legacy ZIP acquired a spectrum.');
    }

    try {
        Package::load($path, filesize($path) - 1);
    } catch (Rejection) {
        continue;
    }
    throw new RuntimeException('The importer accepted a ZIP above the allowed upload size.');
}
echo "Production parser accepted the synthetic legacy and paired ZIPs with exact bytes.\n";
