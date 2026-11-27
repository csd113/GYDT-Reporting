<?php

namespace GYDT\SampleResults;

final class Updater
{
    private const SLUG = 'gydt-sample-results-importer';
    private const PLUGIN = self::SLUG . '/' . self::SLUG . '.php';
    private const URI = 'https://github.com/csd113/GYDT-Reporting';
    private const API = 'https://api.github.com/repos/csd113/GYDT-Reporting/releases/latest';
    private const CACHE = 'gydt_importer_latest_release_v1';
    private const USER_AGENT = 'GYDT-Sample-Results-Importer';
    private const MAX_METADATA = 512 * 1024;
    private const MAX_PACKAGE = 8 * 1024 * 1024;
    private const VERSION = '(?:0|[1-9][0-9]{0,5})\.(?:0|[1-9][0-9]{0,5})\.(?:0|[1-9][0-9]{0,5})';

    public static function register(): void
    {
        add_filter('update_plugins_github.com', [self::class, 'available'], 10, 3);
        add_filter('upgrader_pre_download', [self::class, 'verifyDownload'], PHP_INT_MAX, 4);
    }

    public static function available(mixed $update, array $headers, string $file): mixed
    {
        if ($file !== self::PLUGIN || ($headers['UpdateURI'] ?? '') !== self::URI) {
            return $update;
        }
        $release = self::newerThan($headers['Version'] ?? '');
        if ($release === null) {
            return false;
        }
        return [
            'id' => self::URI,
            'slug' => self::SLUG,
            'version' => $release['version'],
            'url' => self::URI . '/releases/tag/v' . $release['version'],
            'package' => self::url($release['version']),
            'requires_php' => '8.1',
        ];
    }

    private static function version(mixed $version): bool
    {
        return is_string($version) && preg_match('/\A' . self::VERSION . '\z/D', $version) === 1;
    }

    private static function filename(string $version): string
    {
        return 'GYDT-Sample-Results-Importer-v' . $version . '.zip';
    }

    private static function url(string $version): string
    {
        return self::URI . '/releases/download/v' . $version . '/' . self::filename($version);
    }

    private static function newerThan(mixed $installed): ?array
    {
        if (!self::version($installed)) {
            return null;
        }
        $release = self::release();
        return $release !== null && version_compare($release['version'], $installed, '>') ? $release : null;
    }

    private static function release(): ?array
    {
        $cached = get_site_transient(self::CACHE);
        if (is_array($cached)) {
            return self::validAsset($cached) ? $cached : null;
        }
        if ($cached !== false) {
            return null;
        }
        $response = wp_safe_remote_get(self::API, [
            'timeout' => 5,
            'redirection' => 0,
            'user-agent' => self::USER_AGENT,
            'limit_response_size' => self::MAX_METADATA,
            'headers' => ['Accept' => 'application/vnd.github+json'],
        ]);
        $release = is_wp_error($response) || wp_remote_retrieve_response_code($response) !== 200
            ? null : self::parse(wp_remote_retrieve_body($response));
        set_site_transient(self::CACHE, $release ?? [], $release === null ? 15 * MINUTE_IN_SECONDS : 6 * HOUR_IN_SECONDS);
        return $release;
    }

    private static function parse(string $body): ?array
    {
        $data = json_decode($body, true, 12);
        $tag = $data['tag_name'] ?? null;
        if (($data['draft'] ?? null) !== false || ($data['prerelease'] ?? null) !== false
            || empty($data['published_at']) || !is_string($tag) || !str_starts_with($tag, 'v')
            || !self::version($version = substr($tag, 1)) || !is_array($data['assets'] ?? null)) {
            return null;
        }
        $found = null;
        foreach ($data['assets'] as $asset) {
            if (($asset['name'] ?? null) !== self::filename($version)) {
                continue;
            }
            $item = [
                'version' => $version,
                'size' => $asset['size'] ?? null,
                'digest' => $asset['digest'] ?? null,
            ];
            if ($found !== null || ($asset['state'] ?? null) !== 'uploaded'
                || ($asset['browser_download_url'] ?? null) !== self::url($version) || !self::validAsset($item)) {
                return null;
            }
            $found = $item;
        }
        return $found;
    }

    private static function validAsset(array $asset): bool
    {
        return self::version($asset['version'] ?? null) && is_int($asset['size'] ?? null)
            && $asset['size'] >= 22 && $asset['size'] <= self::MAX_PACKAGE
            && is_string($asset['digest'] ?? null)
            && preg_match('/\Asha256:[a-f0-9]{64}\z/D', $asset['digest']) === 1;
    }

    private static function ourUpgrade(array $extra): bool
    {
        return ($extra['plugin'] ?? null) === self::PLUGIN
            && ($extra['type'] ?? 'plugin') === 'plugin' && ($extra['action'] ?? 'update') === 'update';
    }

    public static function verifyDownload(mixed $reply, string $package, \WP_Upgrader $upgrader, array $extra): mixed
    {
        if (!self::ourUpgrade($extra)) {
            return $reply;
        }
        if (is_wp_error($reply)) {
            return $reply;
        }
        $installed = get_file_data(__DIR__ . '/' . self::SLUG . '.php', ['Version' => 'Version']);
        $release = self::newerThan($installed['Version']);
        if ($reply !== false || !$upgrader instanceof \Plugin_Upgrader || $release === null
            || $package !== self::url($release['version'])) {
            return self::unverified();
        }

        $temporary = wp_tempnam(self::filename($release['version']));
        if (!$temporary) {
            return self::unverified();
        }
        $verified = false;
        try {
            $response = self::fetchPackage($package, $temporary, $release['size']);
            clearstatcache(true, $temporary);
            $verified = $response && filesize($temporary) === $release['size']
                && hash_equals($release['digest'], 'sha256:' . hash_file('sha256', $temporary))
                && self::matchesPackage($temporary, $release['version']);
            return $verified ? $temporary : self::unverified(); // Core owns a verified download.
        } finally {
            if (!$verified) {
                wp_delete_file($temporary);
            }
        }
    }

    private static function fetchPackage(string $url, string $file, int $size): bool
    {
        for ($hop = 0; $hop < 3; ++$hop) {
            $response = wp_safe_remote_get($url, [
                'timeout' => 12,
                'redirection' => 0,
                'user-agent' => self::USER_AGENT,
                'decompress' => false,
                'stream' => true,
                'filename' => $file,
                'limit_response_size' => $size + 1,
            ]);
            if (is_wp_error($response)) {
                return false;
            }
            $code = wp_remote_retrieve_response_code($response);
            if ($code === 200) {
                return true;
            }
            if (!in_array($code, [301, 302, 303, 307, 308], true)) {
                return false;
            }
            $location = wp_remote_retrieve_header($response, 'location');
            $parts = is_string($location) ? wp_parse_url($location) : false;
            if (!is_array($parts) || ($parts['scheme'] ?? null) !== 'https'
                || !in_array($parts['host'] ?? null, ['release-assets.githubusercontent.com', 'objects.githubusercontent.com'], true)
                || isset($parts['port']) || isset($parts['user']) || isset($parts['pass']) || isset($parts['fragment'])) {
                return false;
            }
            $url = $location;
        }
        return false;
    }

    private static function matchesPackage(string $file, string $version): bool
    {
        // Inspect only the verified plugin header in memory before the upgrader can
        // deactivate the current plugin. WordPress still handles unpacking/installing.
        require_once ABSPATH . 'wp-admin/includes/class-pclzip.php';
        $archive = new \PclZip($file);
        $items = $archive->listContent();
        if (!is_array($items)) {
            return false;
        }
        $main = 0;
        foreach ($items as $item) {
            if (!str_starts_with($item['filename'], self::SLUG . '/')) {
                return false;
            }
            if ($item['filename'] === self::PLUGIN) {
                if ($item['folder'] || $item['size'] < 1 || $item['size'] > 128 * 1024) {
                    return false;
                }
                ++$main;
            }
        }
        if ($main !== 1) {
            return false;
        }
        $item = $archive->extract(PCLZIP_OPT_BY_NAME, self::PLUGIN, PCLZIP_OPT_EXTRACT_AS_STRING);
        $header = $item[0]['content'] ?? null;
        return is_string($header)
            && preg_match('/^\s*\* Version: ([^\r\n]+)\r?$/m', substr($header, 0, 8192), $match) === 1
            && $match[1] === $version;
    }

    private static function unverified(): \WP_Error
    {
        return new \WP_Error('gydt_unverified_update', 'The GYDT importer update could not be verified. Nothing was installed.');
    }
}
