<?php

namespace GYDT\SampleResults;

use DateTimeImmutable;
use DateTimeZone;
use JsonException;
use RuntimeException;
use stdClass;

final class Rejection extends RuntimeException
{
}

final class Package
{
    public const CATALOGUE = '501794d785f8719a385dda374e4a47d81792d19135f26e863f214d9d6c4b86f8';

    private const CHOICES = [
        'site' => ['fixture-east', 'fixture-west'],
        'colour' => [
            'Black', 'Blue', 'Light Blue', 'Dark Blue', 'Brown', 'Light Brown', 'Dark Brown', 'Colourless',
            'Green', 'Light Green', 'Dark Green', 'Grey', 'Light Grey', 'Dark Grey', 'Orange', 'Light Orange',
            'Dark Orange', 'Pink', 'Light Pink', 'Dark Pink', 'Purple', 'Light Purple', 'Dark Purple', 'Red',
            'Light Red', 'Dark Red', 'White', 'Yellow', 'Light Yellow', 'Dark Yellow', 'other',
        ],
        'texture' => [
            'Bar', 'Blotter', 'Capsule', 'Chunk', 'Crystal', 'Flake', 'Granules', 'Liquid', 'Organic', 'Paste',
            'Pebble', 'Pill', 'Powder', 'Pressed Pill', 'Residue', 'Rock', 'Tablet', 'other',
        ],
        'pill_shape' => [
            'round', 'rectangle', 'capsule', 'teardrop', 'oval', 'barrel', 'cup', 'heart', 'star', '3-sided',
            '4-sided', '5-sided', '6-sided', '7-sided', '8-sided', 'figure8', 'character', 'unique', 'other',
        ],
        'pre_post_consumption' => ['pre', 'post', 'n/a'],
        'adverse_events' => ['yes', 'no', 'na'],
    ];
    private const REQUIRED_SAMPLE_FIELDS = [
        'submission_number' => 'Submission number',
        'visit' => 'Visit number',
        'analysis_date' => 'Analysis date',
        'collection_date' => 'Collection date',
        'site' => 'Collection location',
        'expected_substance' => 'Expected substance (Sold As)',
        'results' => 'Detected substances',
        'colour' => 'Sample colour',
        'texture' => 'Sample texture',
        'pre_post_consumption' => 'Sample use (Has this sample been used?)',
        'adverse_events' => 'Unexpected effects',
        'client' => 'Code word',
    ];
    private const STRIPS = [
        'fent_strip' => 'Fentanyl strip',
        'benzo' => 'Benzodiazepine strip',
        'medetomidine' => 'Medetomidine strip',
        'lysergamide_strip' => 'Lysergamide (LSD) strip',
    ];

    private function __construct(
        public readonly string $id,
        public readonly array $data,
        public readonly string $fingerprint,
        public readonly string $archiveHash,
        public readonly ?string $photo,
        public readonly DateTimeImmutable $collectionDate
    ) {
    }

    public static function load(string $path, int $limit): self
    {
        $zip = new Archive($path, $limit);
        $manifest = self::object(self::json($zip->read('manifest.json')), 'manifest.json', [
            'format_version' => 'Format version',
            'sample_id' => 'Sample ID',
            'exported_at' => 'Export timestamp',
            'catalogue_version' => 'Catalogue version',
            'files' => 'File list',
        ]);
        if ($manifest['format_version'] !== 1 || $manifest['catalogue_version'] !== 'sha256:' . self::CATALOGUE
            || !self::validSampleId($manifest['sample_id'])
            || !is_string($manifest['exported_at'])
            || !preg_match('/\A(\d{4}-\d{2}-\d{2})T(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\dZ\z/D', $manifest['exported_at'], $timestamp)
            || !self::validDate($timestamp[1], 1)) {
            throw new Rejection('The ZIP manifest has an invalid identifier, timestamp or unsupported version.');
        }

        $members = array_values(array_diff($zip->names(), ['manifest.json']));
        $memberLabels = [];
        foreach ($members as $name) {
            $memberLabels[$name] = $name === 'test_data.json' ? 'test_data.json entry' : 'Sample JPEG entry';
        }
        $files = self::object($manifest['files'], 'File list in manifest.json', $memberLabels);
        $content = [];
        foreach ($memberLabels as $name => $label) {
            $entry = self::object($files[$name], $label . ' in manifest.json', [
                'bytes' => 'Byte count',
                'sha256' => 'SHA-256 checksum',
            ]);
            $content[$name] = $zip->read($name);
            if ($entry['bytes'] !== strlen($content[$name]) || $entry['sha256'] !== hash('sha256', $content[$name])) {
                throw new Rejection('The ZIP contents do not match the manifest.');
            }
        }

        $payload = self::json($content['test_data.json']);
        $data = self::validatePayload($payload);
        $photo = null;
        $photoHash = null;
        $photoNames = array_values(array_diff($members, ['test_data.json']));
        if ($photoNames) {
            $name = $photoNames[0];
            if (pathinfo($name, PATHINFO_FILENAME) !== self::filenameStem($data['submission_number'])) {
                throw new Rejection('The sample photo name does not match the submission number.');
            }
            $photo = $content[$name];
            self::validatePhoto($photo);
            $photoHash = $files[$name]->sha256;
        }

        return new self(
            $manifest['sample_id'],
            $data,
            hash('sha256', self::canonical((object) ['data' => $payload, 'jpeg_sha256' => $photoHash])),
            $zip->fingerprint(),
            $photo,
            self::vancouverMidnight($data['collection_date'])
        );
    }

    public static function validSampleId(mixed $value): bool
    {
        return is_string($value)
            && (bool) preg_match('/\A[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\z/D', $value);
    }

    public static function validDate(mixed $value, int $firstYear = 1000): bool
    {
        return is_string($value) && preg_match('/\A(\d{4})-(\d{2})-(\d{2})\z/D', $value, $parts)
            && (int) $parts[1] >= $firstYear && checkdate((int) $parts[2], (int) $parts[3], (int) $parts[1]);
    }

    private static function vancouverMidnight(string $date): DateTimeImmutable
    {
        $zone = new DateTimeZone('America/Vancouver');
        $wallTime = new DateTimeImmutable($date . 'T00:00:00Z');
        $wallSeconds = $wallTime->getTimestamp();
        $instants = [];
        foreach ($zone->getTransitions($wallSeconds - 86400, $wallSeconds + 86400) as $transition) {
            $time = $wallTime->setTimestamp($wallSeconds - $transition['offset'])->setTimezone($zone);
            if ($time->format('Y-m-d H:i:s') === $date . ' 00:00:00') {
                $instants[$time->getTimestamp()] = $time;
            }
        }
        if (count($instants) !== 1) {
            throw new Rejection('The collection date does not have a unique Vancouver midnight.');
        }
        return reset($instants);
    }

    private static function validatePayload(mixed $payload): array
    {
        $data = self::object($payload, 'Sample in test_data.json', self::REQUIRED_SAMPLE_FIELDS + self::STRIPS, [
            'sold_as_other', 'colour_other', 'texture_other', 'pill_shape_other', 'pill_shape',
            'advertse_events_yes_other', 'notes',
        ]);

        self::text($data['submission_number'], 200, 'Submission number');
        self::text($data['client'], 255, 'Sample code');
        if (!is_int($data['visit']) || $data['visit'] < 1 || $data['visit'] > 2147483647) {
            throw new Rejection('Visit must be a positive integer within the supported range.');
        }
        if (!self::validDate($data['analysis_date']) || !self::validDate($data['collection_date'])) {
            throw new Rejection('Enter valid analysis and collection dates.');
        }
        foreach (self::CHOICES as $key => $choices) {
            if (array_key_exists($key, $data) && !in_array($data[$key], $choices, true)) {
                throw new Rejection('Invalid choice for ' . str_replace('_', ' ', $key) . '.');
            }
        }
        foreach (array_keys(self::STRIPS) as $key) {
            if (!in_array($data[$key], ['Not Used', 'Positive', 'Negative', 'Invalid'], true)) {
                throw new Rejection('A test strip result has an invalid value.');
            }
        }
        self::substance($data['expected_substance'], 'Sold As');
        self::optionalText($data, 'sold_as_other', $data['expected_substance'] === 'other');
        self::optionalText($data, 'colour_other', $data['colour'] === 'other');
        self::optionalText($data, 'texture_other', $data['texture'] === 'other');
        if (array_key_exists('pill_shape', $data) && !in_array($data['texture'], ['Pill', 'Tablet', 'Pressed Pill'], true)) {
            throw new Rejection('Pill shape requires a pill or tablet texture.');
        }
        self::optionalText($data, 'pill_shape_other', ($data['pill_shape'] ?? null) === 'other');
        if ($data['pre_post_consumption'] === 'n/a' && $data['adverse_events'] !== 'na') {
            throw new Rejection('Answer whether the sample was used before supplying an unexpected-effects answer.');
        }
        self::optionalText($data, 'advertse_events_yes_other', $data['adverse_events'] === 'yes');
        self::optionalText($data, 'notes', true, 2000);

        if (!is_array($data['results']) || count($data['results']) < 1 || count($data['results']) > 10) {
            throw new Rejection('Include one to ten detected substance rows.');
        }
        foreach ($data['results'] as $index => $item) {
            $label = 'Row ' . ($index + 1);
            $row = self::object($item, $label . ' of detected substances in test_data.json', [
                'substance' => 'Detected substance',
                'percentage' => 'Percentage',
            ], ['other']);
            if (array_key_exists('other', $row) && $row['other'] === null) {
                self::text(null, 255, 'other');
            }
            $data['results'][$index] = self::detectedRow($row['substance'], $row['percentage'], $row['other'] ?? null, $label);
        }
        return $data;
    }

    public static function detectedRow(mixed $substance, mixed $percentage, mixed $other, string $label): array
    {
        self::substance($substance, $label);
        if ($percentage !== 'blank' && (!is_string($percentage)
            || !preg_match('/\A(0|[1-9][0-9]{0,2})-(0|[1-9][0-9]{0,2})%\z/D', $percentage, $range)
            || (int) $range[1] >= (int) $range[2] || (int) $range[2] > 100)) {
            throw new Rejection($label . ' has an invalid percentage.');
        }
        if ($other !== null) {
            self::optionalText(['other' => $other], 'other', $substance === 'other');
        }
        return ['substance' => $substance, 'percentage' => $percentage, 'other' => $other];
    }

    private static function optionalText(array $data, string $key, bool $allowed, int $bytes = 255): void
    {
        if (array_key_exists($key, $data)) {
            if (!$allowed) {
                throw new Rejection('A conditional text field was supplied without its matching choice.');
            }
            self::text($data[$key], $bytes, str_replace('_', ' ', $key));
        }
    }

    private static function text(mixed $value, int $bytes, string $label): void
    {
        if (!is_string($value) || strlen($value) > $bytes || $value !== trim($value, ' ')
            || preg_match('/[<>\x{0}-\x{1f}\x{7f}-\x{9f}\x{2028}\x{2029}]/u', $value) !== 0
            || preg_match('/\A(?:\p{Z}|\s)*\z/uD', $value) !== 0) {
            throw new Rejection($label . ' must contain nonblank plain text within its size limit.');
        }
    }

    private static function substance(mixed $value, string $label): void
    {
        if (!is_string($value) || (!in_array($value, ['unknown', 'other'], true) && !preg_match('/\A[1-9][0-9]*\z/D', $value))) {
            throw new Rejection($label . ' has an invalid substance selection.');
        }
    }

    private static function filenameStem(string $title): string
    {
        if (preg_match('/\A[A-Za-z0-9][A-Za-z0-9 _.-]{0,79}\z/D', $title)
            && $title === rtrim($title, ' .')
            && !preg_match('/\A(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|\z)/iD', $title)) {
            return $title;
        }
        return 'sample-' . hash('sha256', $title);
    }

    private static function object(mixed $value, string $context, array $required, array $optional = []): array
    {
        if (!$value instanceof stdClass) {
            throw new Rejection($context . ' must be a JSON object.');
        }
        $fields = get_object_vars($value);
        $missing = array_diff_key($required, $fields);
        if ($missing) {
            throw new Rejection($context . ' is missing ' . implode(', ', $missing) . '.');
        }
        if (array_diff(array_keys($fields), array_merge(array_keys($required), $optional))) {
            throw new Rejection($context . ' contains unsupported fields.');
        }
        return $fields;
    }

    private static function json(string $bytes): mixed
    {
        try {
            $value = json_decode($bytes, false, 24, JSON_THROW_ON_ERROR);
        } catch (JsonException) {
            throw new Rejection('The ZIP contains invalid UTF-8 JSON.');
        }

        // Native decoding checks syntax. Check object names separately before trusting its last-value behavior.
        if (preg_match_all('/"(?:[^"\\\\]++|\\\\.)*+"|[{}\[\]:]/s', $bytes, $matches) === false) {
            throw new Rejection('The ZIP contains invalid UTF-8 JSON.');
        }
        $objects = [];
        foreach ($matches[0] as $index => $token) {
            if ($token === '{' || $token === '[') {
                $objects[] = [];
            } elseif ($token === '}' || $token === ']') {
                array_pop($objects);
            } elseif ($token[0] === '"' && ($matches[0][$index + 1] ?? '') === ':') {
                $key = json_decode($token, false, 2, JSON_THROW_ON_ERROR);
                $level = count($objects) - 1;
                if (isset($objects[$level][$key])) {
                    throw new Rejection('ZIP JSON object keys must be unique.');
                }
                $objects[$level][$key] = true;
            }
        }
        return $value;
    }

    private static function canonical(mixed $value): string
    {
        // The validated schema uses ASCII object keys, strings, a bounded integer, arrays and null.
        if ($value instanceof stdClass) {
            $keys = get_object_vars($value);
            ksort($keys, SORT_STRING);
            $properties = [];
            foreach ($keys as $key => $item) {
                $properties[] = self::canonical($key) . ':' . self::canonical($item);
            }
            return '{' . implode(',', $properties) . '}';
        }
        if (is_array($value)) {
            return '[' . implode(',', array_map(self::canonical(...), $value)) . ']';
        }
        return json_encode($value, JSON_THROW_ON_ERROR | JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_LINE_TERMINATORS);
    }

    private static function validatePhoto(string $bytes): void
    {
        if (substr($bytes, 0, 2) !== "\xff\xd8") {
            self::invalidPhoto();
        }
        $size = strlen($bytes);
        $offset = 2;
        $jfif = false;
        $scan = false;
        $hasScan = false;
        while ($offset < $size) {
            if ($scan) {
                $offset += strcspn($bytes, "\xff", $offset);
            }
            if ($offset >= $size || $bytes[$offset++] !== "\xff") {
                self::invalidPhoto();
            }
            $offset += strspn($bytes, "\xff", $offset);
            if ($offset >= $size) {
                self::invalidPhoto();
            }
            $marker = ord($bytes[$offset++]);
            if ($scan && ($marker === 0 || ($marker >= 0xd0 && $marker <= 0xd7))) {
                continue;
            }
            if ($marker === 0xd9 && $offset === $size && $hasScan) {
                self::decodePhoto($bytes);
                return;
            }
            if ($marker <= 1 || ($marker >= 0xd0 && $marker <= 0xd9) || $marker === 0xfe || $offset + 2 > $size) {
                self::invalidPhoto();
            }

            $length = unpack('n', $bytes, $offset)[1];
            if ($length < 2 || $offset + $length > $size) {
                self::invalidPhoto();
            }
            if ($marker >= 0xe0 && $marker <= 0xef) {
                if ($marker !== 0xe0 || $jfif || $length !== 16
                    || substr($bytes, $offset + 2, 5) !== "JFIF\0"
                    || ord($bytes[$offset + 7]) !== 1 || ord($bytes[$offset + 8]) > 2
                    || ord($bytes[$offset + 9]) > 2
                    || substr($bytes, $offset + 10, 2) === "\0\0"
                    || substr($bytes, $offset + 12, 2) === "\0\0"
                    || substr($bytes, $offset + 14, 2) !== "\0\0") {
                    self::invalidPhoto();
                }
                $jfif = true;
            }
            $scan = $marker === 0xda;
            $hasScan = $hasScan || $scan;
            $offset += $length;
        }
        self::invalidPhoto();
    }

    private static function decodePhoto(string $bytes): void
    {
        $info = @getimagesizefromstring($bytes);
        if (!$info || $info[2] !== IMAGETYPE_JPEG || min($info[0], $info[1]) < 1
            || max($info[0], $info[1]) > 4096 || $info[0] * $info[1] > 8388608
            || !function_exists('imagecreatefromstring')) {
            self::invalidPhoto();
        }
        $warning = false;
        $previous = ini_set('gd.jpeg_ignore_warning', '0');
        if ($previous === false) {
            self::invalidPhoto();
        }
        set_error_handler(static function () use (&$warning): bool {
            $warning = true;
            return true;
        });
        try {
            $image = imagecreatefromstring($bytes);
        } finally {
            restore_error_handler();
            ini_set('gd.jpeg_ignore_warning', $previous);
        }
        if ($warning || $image === false || imagesx($image) !== $info[0] || imagesy($image) !== $info[1]) {
            self::invalidPhoto();
        }
    }

    private static function invalidPhoto(): never
    {
        throw new Rejection('The sample photo must be a decodable JPEG within the size limits, without metadata or trailing data.');
    }
}
