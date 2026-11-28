<?php

declare(strict_types=1);

use GYDT\SampleResults\Spectrum;

$allowed = true;

function wp_check_filetype(string $name): array
{
    return ['type' => $GLOBALS['allowed'] && str_ends_with($name, '.0') ? 'application/octet-stream' : false];
}

function wp_doing_ajax(): bool
{
    return false;
}

require $argv[1] . '/spectrum.php';
$resolve = new ReflectionMethod(Spectrum::class, 'acceptValidatedZero');
$zero = ['ext' => '0', 'type' => 'application/octet-stream', 'proper_filename' => false];
$file = '3-990-2026-11-27-810.0';

function check(array $expected, array $actual): void
{
    if ($actual !== $expected) {
        throw new RuntimeException('Unexpected validation change for a native Spectra File.');
    }
}

check(array_replace($zero, ['ext' => '.0']), $resolve->invoke(null, $zero, $file));
check($zero, $resolve->invoke(null, $zero, $file . '.jpg'));
check($zero, Spectrum::manualZeroExtension($zero, '/not-an-http-upload', $file));
foreach ([
    ['type' => false],
    ['ext' => false],
    ['ext' => '1'],
    ['type' => 'text/html'],
    ['proper_filename' => 'alternate.0'],
] as $override) {
    $input = array_replace($zero, $override);
    check($input, $resolve->invoke(null, $input, $file));
}
$allowed = false;
check($zero, $resolve->invoke(null, $zero, $file));
fwrite(STDOUT, "The numeric compatibility fix preserves native MIME validation and site upload rules.\n");
