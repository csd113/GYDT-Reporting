<?php

use GYDT\SampleResults\AcfContract;
use GYDT\SampleResults\Rejection;

if (PHP_SAPI !== 'cli') {
    exit(1);
}

require __DIR__ . '/../gydt-sample-results-importer/contract.php';
require __DIR__ . '/../gydt-sample-results-importer/acf-contract.php';

$select = ['type' => 'select', 'return_format' => 'value'];
$radio = ['type' => 'radio', 'return_format' => 'value', 'choices' => ['Not Used' => 'Blank']];
$checkbox = ['type' => 'checkbox', 'return_format' => 'value', 'choices' => ['other' => 'Other']];
$visit = ['type' => 'number', 'min' => '1', 'max' => '25'];

foreach (['Bar', 'Pill', 'Rock'] as $texture) {
    AcfContract::verify('texture', $select + ['choices' => [$texture => $texture]], $texture);
}
AcfContract::verify('site', $select + ['choices' => ['Sites' => ['fixture-west' => 'Example West']]], 'fixture-west');
AcfContract::verify('fent_strip', $radio, 'Not Used');
AcfContract::verify('adverse_events_yes', $checkbox, ['other']);
AcfContract::verify('adverse_events_yes', $checkbox, []);
AcfContract::verify('visit', $visit, 25);
AcfContract::verify('notes', ['type' => 'text'], 'Synthetic note');

$cases = [
    ['colour', $select + ['choices' => ['White' => 'White']], 'Colourless'],
    ['colour', $select + ['choices' => ['colourless' => 'Colourless']], 'Colourless'],
    ['site', $select + ['choices' => ['fixture-west' => ['fixture-east' => 'Example East']]], 'fixture-west'],
    ['site', $select + ['multiple' => 1, 'choices' => ['fixture-west' => 'Example West']], 'fixture-west'],
    ['fent_strip', array_replace($radio, ['type' => 'select']), 'Not Used'],
    ['fent_strip', array_replace($radio, ['return_format' => 'label']), 'Not Used'],
    ['fent_strip', $radio + ['disabled' => ['Not Used']], 'Not Used'],
    ['adverse_events_yes', array_replace($checkbox, ['choices' => []]), ['other']],
    ['adverse_events_yes', $checkbox + ['required' => 1], []],
    ['visit', $visit, 26],
    ['notes', ['type' => 'textarea'], 'Synthetic note'],
    ['notes', ['type' => 'text', 'maxlength' => 'unavailable'], 'Synthetic note'],
];

foreach ($cases as [$name, $field, $value]) {
    try {
        AcfContract::verify($name, $field, $value);
    } catch (Rejection $error) {
        if (is_string($value) && str_contains($error->getMessage(), $value)) {
            throw new RuntimeException('An ACF configuration error disclosed the submitted value.');
        }
        continue;
    }
    throw new RuntimeException('An incompatible ACF field was accepted: ' . $name);
}

echo "Mapped ACF field types, active choices, legacy textures and configuration drift verified.\n";
