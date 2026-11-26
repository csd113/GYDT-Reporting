<?php

if (PHP_SAPI !== 'cli' || $argc !== 3) {
    exit(1);
}
define('ABSPATH', __DIR__);
define('GYDT_ENABLE_SYNTHETIC_FIXTURE', $argv[2] === 'true');
function wp_get_environment_type(): string
{
    global $argv;
    return $argv[1];
}
function add_action(string $name, mixed $callback): void
{
    $GLOBALS['fixture_actions'][] = $name;
}
function add_filter(string $name, mixed $callback): void
{
    $GLOBALS['fixture_filters'][] = $name;
}
require __DIR__ . '/../gydt-sample-results-importer/gydt-sample-results-importer.php';
$enabled = $argv[1] === 'local' && $argv[2] === 'true';
$actions = $enabled ? ['admin_menu', 'admin_post_gydt_import_sample_results', 'admin_post_gydt_import_history'] : [];
$filters = $enabled ? ['wp_insert_post_empty_content'] : [];
if (($GLOBALS['fixture_actions'] ?? []) !== $actions || ($GLOBALS['fixture_filters'] ?? []) !== $filters) {
    throw new RuntimeException('Unexpected hooks for the synthetic fixture runtime.');
}
echo "Synthetic fixture gate passed for {$argv[1]} / {$argv[2]}.\n";
