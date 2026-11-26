<?php

namespace GYDT\SampleResults;

final class AcfContract
{
    public static function verify(string $name, array $field, mixed $value): void
    {
        $type = match ($name) {
            'visit' => 'number',
            'site', 'sample_collection', 'province', 'city', 'colour', 'texture', 'pill_shape',
                'pre_post_consumption', 'adverse_events' => 'select',
            'fent_strip', 'benzo', 'medetomidine', 'lysergamide_strip' => 'radio',
            'adverse_events_yes' => 'checkbox',
            'sold_as_other', 'colour_other', 'texture_other', 'pill_shape_other', 'advertse_events_yes_other',
                'client', 'notes' => 'text',
            default => null,
        };

        if ($type === null || ($field['type'] ?? null) !== $type || !self::supports($type, $field, $value)) {
            $label = str_replace('_', ' ', $name);
            throw new Rejection('The Test Results editor cannot accept the mapped ' . $label
                . ' field. Ask an administrator to check its configuration.');
        }
    }

    private static function supports(string $type, array $field, mixed $value): bool
    {
        if (in_array($type, ['number', 'text', 'select'], true)
            && (!empty($field['disabled']) || !empty($field['readonly']))) {
            return false;
        }
        if ($type === 'number') {
            $min = $field['min'] ?? null;
            $max = $field['max'] ?? null;
            return is_int($value) && (!is_numeric($min) || $value >= $min) && (!is_numeric($max) || $value <= $max);
        }
        if ($type === 'text') {
            $limit = $field['maxlength'] ?? '';
            return is_string($value) && (!$limit || (is_numeric($limit) && function_exists('acf_strlen')
                && acf_strlen(wp_slash($value)) <= $limit));
        }
        if (($field['return_format'] ?? null) !== 'value' || !is_array($field['choices'] ?? null)
            || ($type === 'select' && !empty($field['multiple']))
            || (!empty($field['disabled']) && !is_array($field['disabled']))) {
            return false;
        }
        if ($type === 'checkbox' && (!is_array($value) || (!$value && !empty($field['required'])))) {
            return false;
        }

        // ACF select AJAX keeps selected top-level choices only. Radio choices are always flat.
        $groups = $type === 'checkbox' || ($type === 'select' && (empty($field['ui']) || empty($field['ajax'])));
        $disabled = (array) ($field['disabled'] ?? []);
        foreach ($type === 'checkbox' ? $value : [$value] as $option) {
            if (!is_string($option) || in_array($option, $disabled, true)
                || !self::hasChoice($field['choices'], $option, $groups)) {
                return false;
            }
        }
        return true;
    }

    private static function hasChoice(array $choices, string $option, bool $groups): bool
    {
        if (isset($choices[$option]) && is_scalar($choices[$option])) {
            return true;
        }
        if ($groups) {
            foreach ($choices as $label) {
                if (is_array($label) && self::hasChoice($label, $option, true)) {
                    return true;
                }
            }
        }
        return false;
    }
}
