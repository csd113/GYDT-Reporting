<?php

namespace GYDT\SampleResults;

use DateTimeImmutable;
use DateTimeZone;
use WP_Post;

final class ImportHistory
{
    public const ACTION = 'gydt_import_history';
    public const OWNER_DAY = '_gydt_import_operator_day';
    private const HIDDEN = 'gydt_import_history_hidden';
    private const PAGE_SIZE = 25;
    private const DISMISS_LIMIT = 200;

    public static function clock(?DateTimeImmutable $now = null): array
    {
        $local = ($now ?? new DateTimeImmutable())->setTimezone(new DateTimeZone('America/Vancouver'));
        return [
            'day' => $local->format('Y-m-d'),
            'seconds_to_midnight' => $local->modify('tomorrow midnight')->getTimestamp() - $local->getTimestamp(),
        ];
    }

    public static function ownerDay(int $operator, string $day): string
    {
        return $operator . ':' . $day;
    }

    public static function receipt(?WP_Post $post): ?array
    {
        $row = self::visibleResult($post);
        if ($row === null) {
            return null;
        }
        $owners = get_post_meta($post->ID, self::OWNER_DAY, false);
        $owner = count($owners) === 1 ? $owners[0] : null;
        $prefix = get_current_user_id() . ':';
        $day = is_string($owner) && str_starts_with($owner, $prefix) ? substr($owner, strlen($prefix)) : '';
        $day = Package::validDate($day) ? $day : null;
        return $row + [
            'key' => wp_create_nonce(self::ACTION . ':' . $post->ID),
            'history_day' => $day,
            'clear_key' => $day === null ? null : wp_create_nonce(self::clearAction($day, $post->ID)),
        ];
    }

    public static function handle(): never
    {
        nocache_headers();
        if (!Import::allowed() || ($_SERVER['REQUEST_METHOD'] ?? '') !== 'POST'
            || !verifiedNonce($_POST['_wpnonce'] ?? null, self::ACTION)) {
            wp_send_json(['message' => 'This history view expired. Reload Import Sample Results to try again.'], 403);
        }

        $operator = get_current_user_id();
        $clock = self::clock();
        $day = $clock['day'];
        $requestedDay = $_POST['day'] ?? null;
        $operation = $_POST['operation'] ?? 'list';
        if (!in_array($operation, ['list', 'dismiss', 'clear'], true)) {
            wp_send_json(['message' => 'Choose a supported history action.'], 400);
        }
        try {
            $changed = false;
            if ($operation !== 'list' && $requestedDay === $day) {
                $changed = self::dismiss(
                    $operator,
                    $day,
                    positiveInteger($_POST['id'] ?? null),
                    $operation === 'clear',
                    $_POST['clear_key'] ?? null
                );
                $clock = self::clock();
                $day = $clock['day'];
            }
            $before = $requestedDay === $day && $operation === 'list' ? positiveInteger($_POST['before'] ?? null) : 0;
            $history = self::page($operator, $day, self::hidden($operator, $day), $before);
            $queue = self::queueStatuses($_POST['receipts'] ?? null);
        } catch (Rejection $error) {
            wp_send_json(['message' => $error->getMessage()], 409);
        } catch (\Throwable) {
            wp_send_json(['message' => 'Couldn’t load upload history. Use Refresh status to try again.'], 503);
        }
        wp_send_json($clock + $history + ['queue' => $queue, 'changed' => $changed && $requestedDay === $day]);
    }

    private static function visibleResult(?WP_Post $post): ?array
    {
        if (!$post || $post->post_type !== 'results'
            || in_array($post->post_status, ['auto-draft', 'inherit', 'trash', 'archive', 'archived'], true)
            || !current_user_can('read_post', $post->ID) || !Import::verifiedCompletion($post->ID)) {
            return null;
        }
        $label = match ($post->post_status) {
            'draft' => 'Draft', 'publish' => 'Published', 'pending' => 'Pending review',
            'future' => 'Scheduled', 'private' => 'Private',
            default => get_post_status_object($post->post_status)?->label ?? 'Other status',
        };
        $review = $post->post_status === 'draft' && current_user_can('edit_post', $post->ID)
            ? get_edit_post_link($post, 'raw') : null;
        return [
            'id' => $post->ID,
            'title' => $post->post_title,
            'status' => $label,
            'review_url' => $review ? (esc_url_raw($review) ?: null) : null,
        ];
    }

    private static function queueStatuses(mixed $raw): array
    {
        $receipts = is_string($raw) && strlen($raw) <= 2048 ? json_decode(wp_unslash($raw), true, 3) : null;
        if (!is_array($receipts) || count($receipts) > 20) {
            return [];
        }
        $rows = [];
        foreach ($receipts as $rawId => $key) {
            $id = positiveInteger((string) $rawId);
            if ($id && verifiedNonce($key, self::ACTION . ':' . $id)) {
                if ($row = self::visibleResult(get_post($id))) {
                    $rows[] = $row;
                }
            }
        }
        return $rows;
    }

    private static function hidden(int $operator, string $day): array
    {
        $state = get_user_option(self::HIDDEN, $operator);
        if (!is_array($state) || ($state['day'] ?? null) !== $day) {
            return ['day' => $day, 'through' => 0, 'ids' => []];
        }
        return $state;
    }

    private static function page(int $operator, string $day, array $hidden, int $before): array
    {
        global $wpdb;
        $parameters = [self::OWNER_DAY, self::ownerDay($operator, $day), $hidden['through']];
        $range = '';
        if ($before) {
            $range = ' AND p.ID < %d';
            $parameters[] = $before;
        }
        if ($hidden['ids']) {
            $range .= ' AND p.ID NOT IN (' . implode(',', array_fill(0, count($hidden['ids']), '%d')) . ')';
            array_push($parameters, ...$hidden['ids']);
        }
        $parameters[] = self::PAGE_SIZE + 1;
        // The operator/day marker is part of the transaction that certifies a completed import.
        $ids = $wpdb->get_col($wpdb->prepare(
            "SELECT DISTINCT p.ID FROM $wpdb->postmeta i
             INNER JOIN $wpdb->posts p ON p.ID = i.post_id
             WHERE i.meta_key = %s AND BINARY i.meta_value = BINARY %s AND p.post_type = 'results' AND p.ID > %d$range
             ORDER BY p.ID DESC LIMIT %d",
            $parameters
        ));
        if ($wpdb->last_error !== '') {
            throw new Rejection('Couldn’t load upload history. Use Refresh status to try again.');
        }
        $page = array_slice($ids, 0, self::PAGE_SIZE);
        $rows = [];
        $owner = [self::ownerDay($operator, $day)];
        foreach ($page as $id) {
            if (get_post_meta((int) $id, self::OWNER_DAY, false) === $owner
                && ($row = self::visibleResult(get_post((int) $id)))) {
                $rows[] = $row;
            }
        }
        $through = $rows[0]['id'] ?? 0;
        return [
            'rows' => $rows,
            'next_before' => count($ids) > self::PAGE_SIZE ? (int) end($page) : 0,
            'clear' => $through ? ['id' => $through, 'key' => wp_create_nonce(self::clearAction($day, $through))] : null,
        ];
    }

    private static function clearAction(string $day, int $id): string
    {
        return self::ACTION . ':clear:' . $day . ':' . $id;
    }

    private static function dismiss(int $operator, string $day, int $id, bool $clear, mixed $key): bool
    {
        global $wpdb;
        if (!$id || ($clear && !verifiedNonce($key, self::clearAction($day, $id)))) {
            throw new Rejection('This history action expired. Use Refresh status before trying again.');
        }
        $lock = 'gydt-history:' . substr(hash('sha256', DB_NAME . ':' . $wpdb->posts . ':' . $operator), 0, 48);
        $retries = $wpdb->reconnect_retries;
        $wpdb->reconnect_retries = 0;
        try {
            if ((string) $wpdb->get_var($wpdb->prepare('SELECT GET_LOCK(%s, 2)', $lock)) !== '1') {
                throw new Rejection('WordPress is updating your upload list. Try again shortly.');
            }
            try {
                if (self::clock()['day'] !== $day) {
                    return false;
                }
                // Clear uses a verified snapshot. The result at its boundary may have since disappeared.
                if (!$clear && (get_post_meta($id, self::OWNER_DAY, false) !== [self::ownerDay($operator, $day)]
                    || !self::visibleResult(get_post($id)))) {
                    return true;
                }
                wp_cache_delete($operator, 'user_meta');
                $state = self::hidden($operator, $day);
                $next = $state;
                if ($clear) {
                    $next['through'] = max($state['through'], $id);
                    $next['ids'] = array_values(array_filter($state['ids'], static fn (int $item): bool => $item > $next['through']));
                } elseif ($id > $state['through'] && !in_array($id, $state['ids'], true)) {
                    if (count($state['ids']) >= self::DISMISS_LIMIT) {
                        throw new Rejection('Use Clear list to hide the rest of today’s uploads.');
                    }
                    $next['ids'][] = $id;
                }
                if ($state !== $next) {
                    if (self::clock()['day'] !== $day) {
                        return false;
                    }
                    if (!update_user_option($operator, self::HIDDEN, $next)) {
                        throw new Rejection('Couldn’t update your upload list. Try again.');
                    }
                }
                return true;
            } finally {
                $wpdb->get_var($wpdb->prepare('SELECT RELEASE_LOCK(%s)', $lock));
            }
        } finally {
            $wpdb->reconnect_retries = $retries;
        }
    }
}
