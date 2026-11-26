<?php

namespace GYDT\SampleResults;

use DateTimeZone;

final class Import
{
    public const ID = '_gydt_sample_id';
    private const FINGERPRINT = '_gydt_import_fingerprint';
    private const COMPLETION = '_gydt_import_completion';
    private const SCOPE = '_gydt_import_scope';
    private const CUSTOM_TITLE = '_gydt_import_custom_title';
    private const ANALYSIS_DATE = '_gydt_analysis_date';
    private const ACF_FIELDS = [
        'visit' => 'field_example_0016',
        'site' => 'field_example_0008',
        'sample_collection' => 'field_example_0007',
        'province' => 'field_example_0009',
        'city' => 'field_example_0004',
        'sold_as_other' => 'field_example_0022',
        'colour' => 'field_example_0017',
        'colour_other' => 'field_example_0005',
        'texture' => 'field_example_0013',
        'texture_other' => 'field_example_0021',
        'pill_shape' => 'field_example_0003',
        'pill_shape_other' => 'field_example_0014',
        'pre_post_consumption' => 'field_example_0011',
        'adverse_events' => 'field_example_0002',
        'adverse_events_yes' => 'field_example_0018',
        'advertse_events_yes_other' => 'field_example_0012',
        'fent_strip' => 'field_example_0015',
        'benzo' => 'field_example_0010',
        'medetomidine' => 'field_example_0006',
        'lysergamide_strip' => 'field_example_0019',
        'client' => 'field_example_0001',
        'notes' => 'field_example_0020',
    ];

    public static function allowed(): bool
    {
        $results = get_post_type_object('results');
        return Pilot::local() && is_user_logged_in() && $results
            && current_user_can($results->cap->create_posts) && current_user_can($results->cap->edit_posts);
    }

    public static function allowSavedZero(mixed $empty, array $fields): mixed
    {
        if (!$empty || ($fields['post_type'] ?? '') !== 'results' || ($fields['post_title'] ?? '') !== '0'
            || empty($fields['ID'])) {
            return $empty;
        }
        $post = get_post((int) $fields['ID']);
        // Staff must be able to save editor fields while keeping an imported title of "0".
        return $post && $post->post_type === 'results' && $post->post_title === '0'
            && metadata_exists('post', $post->ID, self::ID) && current_user_can('edit_post', $post->ID)
            ? false : $empty;
    }

    public static function run(Package $package, bool $preview): array
    {
        global $wpdb;
        if (!self::allowed()) {
            throw new Rejection('Import is not available to this account or site. No draft was saved.');
        }

        $suppress = $wpdb->suppress_errors();
        try {
            if (!SubmissionLock::acquire()) {
                throw new Rejection('Test Results is busy or unavailable. No draft was saved. Try the same ZIP again.');
            }
            try {
                if (!SubmissionLock::inAutocommit()) {
                    throw new UnknownOutcome();
                }
                $existing = self::existing($package);
                if ($existing !== null) {
                    return $existing;
                }
                $fields = self::preflight($package, $preview);
                return $preview ? ['ready', null] : self::store($package, $fields);
            } finally {
                SubmissionLock::release();
            }
        } finally {
            $wpdb->suppress_errors($suppress);
        }
    }

    private static function preflight(Package $package, bool $preview): array
    {
        if (wp_timezone_string() !== 'America/Vancouver') {
            throw new Rejection('No draft was saved. Ask an administrator to check the site’s Vancouver timezone.');
        }
        self::checkCatalogue($package->data);
        if ($package->photo !== null && !current_user_can('upload_files')) {
            throw new Rejection($preview
                ? 'This account can’t add Featured Images. No import was attempted.'
                : 'No draft was saved. Ask a staff member who can add Featured Images to import this ZIP.');
        }
        if ($package->photo !== null
            && (strlen($package->photo) > wp_max_upload_size()
                || wp_check_filetype('sample.jpg')['type'] !== 'image/jpeg')) {
            throw new Rejection('The photo is over the site’s size limit or JPEGs are disabled. No draft was saved.');
        }
        return self::acfValues($package->data);
    }

    private static function checkCatalogue(array $data): void
    {
        self::checkSubstances(array_merge([$data['expected_substance']], array_column($data['results'], 'substance')));
    }

    private static function checkSubstances(array $ids): void
    {
        global $wpdb;
        $ids = array_diff(array_unique($ids), ['unknown', 'other']);
        if (!$ids) {
            return;
        }

        $source = __DIR__ . '/substances-catalog.xml';
        if (!function_exists('simplexml_load_file') || !is_readable($source)
            || hash_file('sha256', $source) !== Package::CATALOGUE) {
            throw new Rejection('The approved substance catalogue is unavailable.');
        }
        $xml = @simplexml_load_file($source, \SimpleXMLElement::class, LIBXML_NONET);
        if (!$xml) {
            throw new Rejection('The approved substance catalogue is unavailable.');
        }
        $mapping = [];
        foreach ($xml->channel->item as $item) {
            $record = $item->children('http://wordpress.org/export/1.2/');
            $mapping[(string) $record->post_id] = (string) $record->post_name;
        }
        foreach ($ids as $id) {
            if (!isset($mapping[$id])) {
                throw new Rejection('A substance selection is outside the approved catalogue.');
            }
            $row = $wpdb->get_row($wpdb->prepare(
                "SELECT post_type, post_status, post_name FROM $wpdb->posts WHERE ID = %d",
                (int) $id
            ));
            self::checkQuery();
            if (!$row || $row->post_type !== 'substances' || $row->post_status !== 'publish'
                || $row->post_name !== $mapping[$id]) {
                throw new Rejection('A selected substance is missing, unpublished or has a mismatched catalogue slug.');
            }
        }
    }

    private static function acfValues(array $data): array
    {
        if (!function_exists('acf_get_field_groups') || !function_exists('acf_get_fields') || !function_exists('update_field')) {
            throw new Rejection('The Test Results editor fields are unavailable.');
        }
        $data += ['sample_collection' => 'walk-in', 'province' => 'BC', 'city' => 'Vancouver'];
        if ($data['adverse_events'] === 'yes') {
            $data['adverse_events_yes'] = isset($data['advertse_events_yes_other']) ? ['other'] : [];
        }
        $fields = array_intersect_key($data, self::ACF_FIELDS);
        $active = [];
        foreach (acf_get_field_groups(['post_type' => 'results', 'post_status' => 'draft', 'post_id' => 0]) as $group) {
            foreach (acf_get_fields($group) ?: [] as $field) {
                $active[$field['key']] = $field;
            }
        }
        foreach ($fields as $name => $value) {
            $field = $active[self::ACF_FIELDS[$name]] ?? null;
            if (!$field || ($field['name'] ?? null) !== $name) {
                throw new Rejection('The Test Results editor fields do not match the import contract.');
            }
            AcfContract::verify($name, $field, $value);
        }
        return $fields;
    }

    private static function store(Package $package, array $fields): array
    {
        global $wpdb;
        $postId = 0;
        $attachment = 0;
        $uploadPath = null;
        $inserted = [];
        self::requireTransactions();
        $origin = 'gydt-import:' . bin2hex(random_bytes(24));
        $draft = [
            'post_type' => 'results',
            'post_status' => 'draft',
            'post_author' => get_current_user_id(),
            'post_title' => $package->data['submission_number'],
            'post_date' => $package->collectionDate->format('Y-m-d H:i:s'),
            'post_date_gmt' => $package->collectionDate->setTimezone(new DateTimeZone('UTC'))->format('Y-m-d H:i:s'),
            'post_content_filtered' => $origin,
        ];
        $meta = [
            'expected_substance' => $package->data['expected_substance'],
            'results' => $package->data['results'],
            self::ID => $package->id,
            self::SCOPE => self::scope($package->data),
            self::ANALYSIS_DATE => $package->data['analysis_date'],
            self::FINGERPRINT => $package->fingerprint,
            ImportHistory::OWNER_DAY => ImportHistory::ownerDay($draft['post_author'], ImportHistory::clock()['day']),
        ];
        if (!self::standardTitle($draft['post_title'])) {
            $meta[self::CUSTOM_TITLE] = self::customTitle($draft['post_title'], $package->data['site']);
        }
        $transactionStarted = false;
        $savepoint = false;
        $commitSent = false;
        // WordPress normally runs this even when PHP exits before the request can catch a failure.
        // Never delete files on an uncertain commit, or if this transaction can no longer be proved.
        $shutdown = static function () use ($origin, &$transactionStarted, &$savepoint, &$commitSent, &$inserted, &$uploadPath): void {
            if (!$transactionStarted || $commitSent) {
                return;
            }
            try {
                if (self::rollbackWrite($origin, $savepoint, $inserted) && $uploadPath !== null) {
                    self::removeRolledBackPhoto($uploadPath);
                }
            } finally {
                SubmissionLock::release();
            }
        };
        add_action('shutdown', $shutdown, PHP_INT_MIN);
        try {
            if (!SubmissionLock::begin()) {
                throw new Rejection('The importer could not start a database transaction.');
            }
            $transactionStarted = true;
            if ($wpdb->query('SAVEPOINT gydt_import_write') === false) {
                throw new Rejection('The importer could not verify a database transaction.');
            }
            $savepoint = true;
            $postId = self::insertPost($draft, $inserted);
            if (!current_user_can('edit_post', $postId)) {
                throw new Rejection('Your account cannot edit the imported draft.');
            }
            self::writeAcf($postId, $fields);
            foreach ($meta as $name => $value) {
                self::writeMeta($postId, $name, $value);
            }
            if ($package->photo !== null) {
                self::attachPhoto($postId, $draft, $package->photo, $attachment, $uploadPath, $inserted);
            }

            if (!self::lockedTransaction()) {
                throw new UnknownOutcome();
            }
            // Finish normal write hooks before certifying the stored draft and native attachment.
            self::verifyDraft($postId, $draft, $fields, $meta, $attachment);
            if ($attachment) {
                self::verifyPhoto($attachment, $postId, $draft['post_author'], $uploadPath, $package->photo);
            }
            self::recordCompletion($postId, $package->fingerprint);
            if (!self::hasCompletion($postId, $package->fingerprint)) {
                throw new Rejection('WordPress couldn’t verify completion. No draft was saved.');
            }
            // Verify the original transaction was not replaced by a hook after an implicit commit.
            if (!self::lockedTransaction() || $wpdb->query('RELEASE SAVEPOINT gydt_import_write') === false) {
                throw new UnknownOutcome();
            }
            $commitSent = true;
            if (!SubmissionLock::commit()) {
                throw new UnknownOutcome();
            }
        } catch (\Throwable $failure) {
            $rolledBack = !$transactionStarted && SubmissionLock::inAutocommit();
            if ($transactionStarted && !$commitSent) {
                $rolledBack = self::rollbackWrite($origin, $savepoint, $inserted);
            } elseif (!$rolledBack) {
                SubmissionLock::rollback();
            }
            if ($rolledBack && !$commitSent) {
                if ($uploadPath !== null && !self::removeRolledBackPhoto($uploadPath)) {
                    throw new Rejection('No draft was saved. Ask an administrator to check the Media Library upload before retrying.');
                }
                if ($failure instanceof Rejection || $failure instanceof Conflict) {
                    throw $failure;
                }
                throw new Rejection('WordPress could not save this ZIP. No draft was saved.');
            }
            // An inactive transaction or a COMMIT sent without confirmation may already be durable.
            throw new UnknownOutcome();
        } finally {
            remove_action('shutdown', $shutdown, PHP_INT_MIN);
        }
        if (!SubmissionLock::inAutocommit() || !self::hasCompletion($postId, $package->fingerprint)) {
            throw new UnknownOutcome();
        }
        return ['imported', $postId];
    }

    private static function rollbackWrite(string $origin, bool $savepoint, array $inserted): bool
    {
        global $wpdb;
        $original = self::lockedTransaction();
        if ($original) {
            $pendingIds = $wpdb->get_col($wpdb->prepare(
                "SELECT ID FROM $wpdb->posts WHERE BINARY post_content_filtered = BINARY %s AND post_type = 'results'",
                $origin
            ));
            $original = $wpdb->last_error === ''
                && (!$savepoint || $wpdb->query('RELEASE SAVEPOINT gydt_import_write') !== false);
        }
        if (!SubmissionLock::rollback() || !$original) {
            return false;
        }
        foreach (array_unique(array_merge($inserted, array_map('intval', $pendingIds ?? []))) as $id) {
            self::clearCaches($id);
        }
        return true;
    }

    public static function lockedTransaction(): bool
    {
        return SubmissionLock::inTransaction();
    }

    private static function writeAcf(int $postId, array $fields): void
    {
        foreach ($fields as $name => $value) {
            $key = self::ACF_FIELDS[$name];
            update_field($key, wp_slash($value), $postId);
            if (get_post_meta($postId, $name, true) !== (is_int($value) ? (string) $value : $value)
                || get_post_meta($postId, '_' . $name, true) !== $key) {
                throw new Rejection('A Test Results editor field could not be saved exactly.');
            }
        }
    }

    public static function requireTransactions(): void
    {
        global $wpdb;
        // WordPress touches the term tables for configured default terms on insertion.
        $tables = [$wpdb->posts, $wpdb->postmeta, $wpdb->terms, $wpdb->term_taxonomy, $wpdb->term_relationships, $wpdb->options];
        $engines = $wpdb->get_col($wpdb->prepare(
            'SELECT ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME IN (%s, %s, %s, %s, %s, %s)',
            ...$tables
        ));
        if (wp_using_ext_object_cache() || count($engines) !== count($tables) || array_diff($engines, ['InnoDB'])
            || !SubmissionLock::inAutocommit()) {
            throw new Rejection('The WordPress database cannot guarantee a transactional import.');
        }
    }

    private static function attachPhoto(
        int $postId, array $draft, string $bytes, int &$attachment, ?string &$uploadPath, array &$inserted
    ): void {
        if (!self::lockedTransaction() || !current_user_can('edit_post', $postId)
            || !current_user_can('upload_files')) {
            throw new Rejection('WordPress didn’t allow the Featured Image to be saved.');
        }
        require_once ABSPATH . 'wp-admin/includes/file.php';
        require_once ABSPATH . 'wp-admin/includes/image.php';

        // Use the normal WordPress upload path and image handling. The unique basename
        // confines cleanup to this upload and its native derivatives if the draft rolls back.
        $uploads = wp_upload_dir($draft['post_date']);
        $directory = empty($uploads['error']) ? realpath($uploads['path']) : false;
        $name = 'gydt-sample-' . wp_generate_uuid4() . '.jpg';
        if ($directory === false || self::photoFiles($directory . '/' . $name) !== []) {
            throw new Rejection('The WordPress uploads folder isn’t available. No draft was saved.');
        }
        $tmp = wp_tempnam($name);
        if (!$tmp) {
            throw new Rejection('WordPress couldn’t prepare the sample JPG. No draft was saved.');
        }
        $expected = $directory . '/' . $name;
        $verifyDestination = static function (mixed $move, array $upload, string $path) use ($tmp, $expected): mixed {
            if (($upload['tmp_name'] ?? null) === $tmp
                && (realpath(dirname($path)) . '/' . basename($path) !== $expected || $move !== null)) {
                throw new Rejection('WordPress couldn’t use its normal upload location. No draft was saved.');
            }
            return $move;
        };
        try {
            if (file_put_contents($tmp, $bytes) !== strlen($bytes)) {
                throw new Rejection('WordPress couldn’t prepare the complete sample JPG. No draft was saved.');
            }
            $uploadPath = $expected;
            $upload = ['name' => $name, 'tmp_name' => $tmp, 'error' => UPLOAD_ERR_OK, 'size' => strlen($bytes)];
            add_filter('pre_move_uploaded_file', $verifyDestination, PHP_INT_MAX, 3);
            try {
                $file = wp_handle_sideload($upload, ['test_form' => false], $draft['post_date']);
            } finally {
                remove_filter('pre_move_uploaded_file', $verifyDestination, PHP_INT_MAX);
            }
            if (($file['type'] ?? null) !== 'image/jpeg'
                || realpath($file['file'] ?? '') !== $uploadPath || empty($file['url'])) {
                throw new Rejection('WordPress couldn’t upload the sample JPG. No draft was saved.');
            }
            $id = wp_insert_attachment([
                'post_mime_type' => $file['type'],
                'guid' => $file['url'],
                'post_title' => $draft['post_title'] . ' sample photo',
                'post_status' => 'inherit',
                'post_author' => $draft['post_author'],
            ], $file['file'], $postId, true);
            if (is_wp_error($id) || !$id) {
                throw new Rejection('WordPress couldn’t add the JPG to the Media Library. No draft was saved.');
            }
            $attachment = $id;
            $inserted[] = $id;
            wp_update_attachment_metadata($id, wp_generate_attachment_metadata($id, $file['file']));
            if (!set_post_thumbnail($postId, $id)) {
                throw new Rejection('WordPress couldn’t set the Featured Image. No draft was saved.');
            }
        } finally {
            if (is_file($tmp)) {
                wp_delete_file($tmp);
            }
        }
    }

    private static function verifyPhoto(int $id, int $parent, int $author, string $path, string $bytes): void
    {
        $metadata = wp_get_attachment_metadata($id, true);
        $attached = self::metaValues($id, '_wp_attached_file');
        $original = wp_get_original_image_path($id, true);
        if (count($attached) !== 1 || !is_array($metadata) || ($metadata['file'] ?? null) !== $attached[0]
            || empty($metadata['width']) || empty($metadata['height'])
            || !$original || realpath($original) !== $path || is_link($path)
            || hash_file('sha256', $path) !== hash('sha256', $bytes)) {
            throw new Rejection('WordPress couldn’t verify the original sample JPG. No draft was saved.');
        }
        self::verifyStored($id, [
            'post_type' => 'attachment', 'post_status' => 'inherit', 'post_parent' => $parent,
            'post_author' => $author, 'post_mime_type' => 'image/jpeg',
        ], ['_wp_attached_file' => $attached[0], '_wp_attachment_metadata' => $metadata], []);
    }

    private static function photoFiles(string $path): ?array
    {
        $stem = preg_quote(pathinfo($path, PATHINFO_FILENAME), '/');
        $directory = dirname($path);
        if (realpath($directory) !== $directory) {
            return null;
        }
        $entries = glob(addcslashes(substr($path, 0, -4), '\\*?[]') . '*');
        if ($entries === false) {
            return null;
        }
        $files = [];
        foreach ($entries as $entry) {
            if (is_link($entry) || !is_file($entry)
                || !preg_match('/\A' . $stem . '(?:-scaled|-rotated)?(?:-[0-9]+x[0-9]+)?\.(?:jpg|jpeg|webp|avif)\z/D', basename($entry))) {
                return null;
            }
            $files[] = $entry;
        }
        return $files;
    }

    private static function removeRolledBackPhoto(string $path): bool
    {
        $files = self::photoFiles($path);
        if ($files === null) {
            return false;
        }
        foreach ($files as $file) {
            wp_delete_file_from_directory($file, dirname($path));
        }
        return self::photoFiles($path) === [];
    }

    private static function insertPost(array $fields, array &$inserted): int
    {
        // Keep this import's status and exact contract fields through WordPress insertion filters.
        $filter = 'wp_insert_post_data';
        $marker = new \stdClass();
        $slashed = wp_slash($fields);
        $guard = static fn(array $data, array $post): array => ($post['_gydt_insert'] ?? null) === $marker
            ? array_replace($data, $slashed) : $data;
        // Core treats a title of "0" as false in its otherwise-empty post check.
        $allowZero = static fn(mixed $empty, array $post): mixed => ($post['_gydt_insert'] ?? null) === $marker
            ? false : $empty;
        $track = static function (int $id, \WP_Post $post) use ($fields, &$inserted): void {
            if ($post->post_type === $fields['post_type'] && $post->post_content_filtered === $fields['post_content_filtered']) {
                $inserted[] = $id;
            }
        };
        add_filter($filter, $guard, PHP_INT_MAX, 2);
        add_action('clean_post_cache', $track, PHP_INT_MIN, 2);
        $zero = $fields['post_type'] === 'results' && $fields['post_title'] === '0';
        if ($zero) {
            add_filter('wp_insert_post_empty_content', $allowZero, PHP_INT_MAX, 2);
        }
        try {
            $id = wp_insert_post($slashed + ['_gydt_insert' => $marker], true, false);
        } finally {
            remove_filter($filter, $guard, PHP_INT_MAX);
            remove_action('clean_post_cache', $track, PHP_INT_MIN);
            if ($zero) {
                remove_filter('wp_insert_post_empty_content', $allowZero, PHP_INT_MAX);
            }
        }
        if (is_wp_error($id) || !$id) {
            throw new Rejection('WordPress could not create the import draft.');
        }
        $inserted[] = $id;
        return $id;
    }

    private static function writeMeta(int $id, string $name, mixed $value): void
    {
        if (!add_post_meta($id, $name, wp_slash($value), true) || get_post_meta($id, $name, true) !== $value) {
            throw new Rejection('WordPress could not save the import metadata.');
        }
    }

    private static function verifyDraft(int $id, array $post, array $acf, array $meta, int $attachment): void
    {
        $absent = [self::COMPLETION];
        if (self::standardTitle($post['post_title'])) {
            $absent[] = self::CUSTOM_TITLE;
        }
        foreach (self::ACF_FIELDS as $name => $key) {
            if (array_key_exists($name, $acf)) {
                $meta[$name] = $acf[$name];
                $meta['_' . $name] = $key;
            } else {
                $absent[] = $name;
                $absent[] = '_' . $name;
            }
        }
        if ($attachment) {
            $meta['_thumbnail_id'] = $attachment;
        } else {
            $absent[] = '_thumbnail_id';
        }
        self::verifyStored($id, $post, $meta, $absent);
        self::verifyNoVisitTerms($id);
    }

    private static function verifyNoVisitTerms(int $id): void
    {
        global $wpdb;
        $relationship = $wpdb->get_var($wpdb->prepare(
            "SELECT r.term_taxonomy_id FROM $wpdb->term_relationships r
             INNER JOIN $wpdb->term_taxonomy t ON t.term_taxonomy_id = r.term_taxonomy_id
             WHERE r.object_id = %d AND t.taxonomy = %s LIMIT 1",
            $id,
            'visit'
        ));
        self::checkQuery();
        if ($relationship !== null) {
            throw new Rejection('WordPress assigned an unexpected Visit term during import.');
        }
    }

    private static function verifyStored(int $id, array $post, array $meta, array $absent): void
    {
        global $wpdb;
        $row = $wpdb->get_row($wpdb->prepare(
            "SELECT post_type, post_status, post_author, post_title, post_date, post_date_gmt, post_parent,
                    post_mime_type, post_content_filtered FROM $wpdb->posts WHERE ID = %d",
            $id
        ));
        self::checkQuery();
        foreach ($post as $name => $expected) {
            if (!$row || (string) $row->$name !== (string) $expected) {
                throw new Rejection('WordPress changed a required draft or attachment field during import.');
            }
        }
        $rows = $wpdb->get_results($wpdb->prepare(
            "SELECT meta_key, meta_value FROM $wpdb->postmeta WHERE post_id = %d",
            $id
        ));
        self::checkQuery();
        $stored = [];
        foreach ($rows as $row) {
            $stored[$row->meta_key][] = $row->meta_value;
        }
        foreach ($meta as $name => $expected) {
            if (($stored[$name] ?? []) !== [(string) maybe_serialize($expected)]) {
                throw new Rejection('WordPress changed required Test Results or attachment data during import.');
            }
        }
        if (array_intersect_key($stored, array_flip($absent))) {
            throw new Rejection('WordPress added an unexpected Test Results or attachment field during import.');
        }
    }

    private static function recordCompletion(int $id, string $fingerprint): void
    {
        global $wpdb;
        // All normal write hooks have already returned. Do not run another metadata hook after final verification.
        if ($wpdb->insert($wpdb->postmeta, [
            'post_id' => $id,
            'meta_key' => self::COMPLETION,
            'meta_value' => '1:' . $fingerprint,
        ], ['%d', '%s', '%s']) !== 1) {
            throw new Rejection('WordPress could not record completion of the verified draft.');
        }
        wp_cache_delete($id, 'post_meta');
    }

    private static function hasCompletion(int $id, string $fingerprint): bool
    {
        return self::metaValues($id, self::COMPLETION) === ['1:' . $fingerprint];
    }

    private static function completedFingerprint(int $id): ?string
    {
        $fingerprints = self::metaValues($id, self::FINGERPRINT);
        $fingerprint = count($fingerprints) === 1 ? $fingerprints[0] : null;
        return is_string($fingerprint) && preg_match('/\A[0-9a-f]{64}\z/D', $fingerprint)
            && self::hasCompletion($id, $fingerprint) ? $fingerprint : null;
    }

    public static function verifiedCompletion(int $id): bool
    {
        $ids = self::metaValues($id, self::ID);
        return count($ids) === 1 && Package::validSampleId($ids[0])
            && array_map('intval', self::matchingMeta(self::ID, $ids[0])) === [$id]
            && self::completedFingerprint($id) !== null;
    }

    public static function clearCaches(int $id): void
    {
        wp_cache_delete($id, 'posts');
        wp_cache_delete('post_parent:' . $id, 'posts');
        wp_cache_delete($id, 'post_meta');
        wp_cache_set_posts_last_changed();
        if (function_exists('acf_flush_value_cache')) {
            foreach (array_keys(self::ACF_FIELDS) as $name) {
                acf_flush_value_cache($id, $name);
            }
        }
    }

    private static function scope(array $data): string
    {
        $title = $data['submission_number'];
        $parts = self::standardTitle($title) ? [$title] : [$title, $data['site'], $data['analysis_date']];
        return hash('sha256', json_encode($parts, JSON_THROW_ON_ERROR));
    }

    private static function customTitle(string $title, string $site): string
    {
        return hash('sha256', json_encode([$title, $site], JSON_THROW_ON_ERROR));
    }

    public static function standardTitle(string $title): bool
    {
        return preg_match('/\A[0-9]+-[0-9]{3}-([0-9]{4}-[0-9]{2}-[0-9]{2})-([0-9]{3,})\z/D', $title, $parts)
            && Package::validDate($parts[1]) && ltrim($parts[2], '0') !== '';
    }

    private static function existing(Package $package): ?array
    {
        $byId = self::matchingMeta(self::ID, $package->id);
        if (count($byId) > 1) {
            self::conflict();
        }
        if ($byId) {
            $id = (int) $byId[0];
            if (self::metaValues($id, self::ID) !== [$package->id]
                || self::metaValues($id, self::FINGERPRINT) !== [$package->fingerprint]) {
                self::conflict($id);
            }
            $post = self::resultRow($id);
            // A matching ZIP reserves the submission even if the caller
            // cannot inspect the Result. Do not reveal its lifecycle state.
            if (!$post || !current_user_can('read_post', $id) || !current_user_can('edit_post', $id)) {
                self::conflict();
            }
            if ($post->post_status === 'auto-draft') {
                throw new UnknownOutcome();
            }
            $original = $package->data;
            $reserved = array_diff(array_map('intval', self::reservedResults(self::scope($original))), [$id]);
            if ($reserved) {
                self::conflict();
            }
            self::assertTitleAvailable($original['submission_number'], $original['site'], $original['analysis_date'], $id);
            [$site, $date] = self::knownScope($id);
            self::assertCurrentResultClaim($id, $post->post_title, $site, $date);
            if ($post->post_status === 'trash') {
                return ['trash', $id];
            }
            if (self::completedFingerprint($id) !== $package->fingerprint) {
                throw new Conflict($id, 'No new draft was saved. Review the existing Test Result before trying again.');
            }
            return ['duplicate', $id];
        }

        $reserved = self::reservedResults(self::scope($package->data));
        if ($reserved) {
            self::conflict(count($reserved) === 1 ? (int) $reserved[0] : null);
        }

        self::assertTitleAvailable($package->data['submission_number'], $package->data['site'], $package->data['analysis_date']);
        return null;
    }

    private static function assertCurrentResultClaim(int $id, string $title, ?string $site, ?string $date): void
    {
        $sites = $site === null ? ['fixture-east', 'fixture-west'] : [$site];
        $reserved = [];
        if (self::standardTitle($title)) {
            $reserved = self::reservedResults(self::scope(['submission_number' => $title]));
        } else {
            foreach ($sites as $candidateSite) {
                $reserved = array_merge($reserved, $date === null
                    ? self::reservedResults(self::customTitle($title, $candidateSite), true)
                    : self::reservedResults(self::scope([
                        'submission_number' => $title, 'site' => $candidateSite, 'analysis_date' => $date,
                    ])));
            }
            if ($date === null) {
                $reserved = array_merge($reserved, self::legacyCustomClaims($id, $title, $sites));
            }
        }
        $reserved = array_diff(array_unique(array_map('intval', $reserved)), [$id]);
        if ($reserved) {
            self::conflict(count($reserved) === 1 ? reset($reserved) : null);
        }
        self::assertTitleAvailable($title, $site, $date, $id);
    }

    private static function legacyCustomClaims(int $except, string $title, array $sites): array
    {
        global $wpdb;
        $rows = $wpdb->get_results($wpdb->prepare(
            "SELECT p.ID, p.post_title, s.meta_value AS scope, a.meta_value AS analysis_date
             FROM $wpdb->posts p
             INNER JOIN $wpdb->postmeta s ON s.post_id = p.ID AND s.meta_key = %s
             LEFT JOIN $wpdb->postmeta a ON a.post_id = p.ID AND a.meta_key = %s
             WHERE p.post_type = 'results' AND p.ID <> %d AND NOT EXISTS (
                 SELECT 1 FROM $wpdb->postmeta c WHERE c.post_id = p.ID AND c.meta_key = %s
             )",
            self::SCOPE, self::ANALYSIS_DATE, $except, self::CUSTOM_TITLE
        ));
        self::checkQuery();
        $claims = [];
        foreach ($rows as $row) {
            if (self::standardTitle($row->post_title) && $row->scope === self::scope(['submission_number' => $row->post_title])) {
                continue;
            }
            if (!Package::validDate($row->analysis_date)) {
                throw new UnknownOutcome();
            }
            if (self::matchesLegacyScope($title, $sites, $row->analysis_date, $row->scope)) {
                $claims[] = (int) $row->ID;
            }
        }
        return $claims;
    }

    private static function matchesLegacyScope(string $title, array $sites, string $date, string $scope): bool
    {
        foreach ($sites as $site) {
            if ($scope === self::scope(['submission_number' => $title, 'site' => $site, 'analysis_date' => $date])) {
                return true;
            }
        }
        return false;
    }

    private static function reservedResults(string $digest, bool $customTitle = false): array
    {
        return self::matchingMeta($customTitle ? self::CUSTOM_TITLE : self::SCOPE, $digest);
    }

    private static function assertTitleAvailable(string $title, ?string $site, ?string $date, int $except = 0): void
    {
        global $wpdb;
        $matches = $wpdb->get_col($wpdb->prepare(
            "SELECT ID FROM $wpdb->posts WHERE post_type = 'results' AND post_status <> 'auto-draft' AND ID <> %d
             AND post_title = %s AND BINARY post_title = BINARY %s",
            $except,
            $title,
            $title
        ));
        self::checkQuery();
        $standard = self::standardTitle($title);
        $conflicts = [];
        foreach ($matches as $match) {
            $id = (int) $match;
            [$otherSite, $otherDate] = $standard ? [null, null] : self::knownScope($id);
            if ($standard || (($site === null || $otherSite === null || $site === $otherSite)
                && ($date === null || $otherDate === null || $date === $otherDate))) {
                $conflicts[] = $id;
            }
        }
        if ($conflicts) {
            self::conflict(count($conflicts) === 1 ? $conflicts[0] : null);
        }
    }

    private static function resultRow(int $id): ?object
    {
        global $wpdb;
        $row = $wpdb->get_row($wpdb->prepare(
            "SELECT post_title, post_status FROM $wpdb->posts WHERE ID = %d AND post_type = 'results'",
            $id
        ));
        self::checkQuery();
        return $row;
    }

    private static function knownScope(int $id): array
    {
        $sites = self::metaValues($id, 'site');
        $dates = self::metaValues($id, self::ANALYSIS_DATE);
        $date = count($dates) === 1 ? $dates[0] : null;
        // A legacy local post_date tracks publication or collection and cannot prove the analysis day.
        return [count($sites) === 1 ? self::knownSite($sites[0]) : null, Package::validDate($date) ? $date : null];
    }

    private static function knownSite(?string $site): ?string
    {
        return in_array($site, ['fixture-east', 'fixture-west'], true) ? $site : null;
    }

    private static function matchingMeta(string $key, string $value): array
    {
        global $wpdb;
        $results = $wpdb->get_col($wpdb->prepare(
            "SELECT DISTINCT p.ID FROM $wpdb->posts p INNER JOIN $wpdb->postmeta m ON m.post_id = p.ID
             WHERE p.post_type = 'results' AND m.meta_key = %s AND BINARY m.meta_value = BINARY %s",
            $key,
            $value
        ));
        self::checkQuery();
        return $results;
    }

    private static function metaValues(int $id, string $key): array
    {
        global $wpdb;
        $values = $wpdb->get_col($wpdb->prepare(
            "SELECT meta_value FROM $wpdb->postmeta WHERE post_id = %d AND meta_key = %s",
            $id,
            $key
        ));
        self::checkQuery();
        return $values;
    }

    private static function checkQuery(): void
    {
        global $wpdb;
        if ($wpdb->last_error !== '') {
            throw new UnknownOutcome();
        }
    }

    private static function conflict(?int $id = null): never
    {
        throw new Conflict($id);
    }
}

final class Conflict extends \RuntimeException
{
    public function __construct(
        public readonly ?int $postId,
        public readonly ?string $reason = null
    ) {
        parent::__construct($reason ?? 'No new draft was saved. Check the ZIP and ask an administrator if you can’t resolve it.');
    }
}

final class UnknownOutcome extends \RuntimeException
{
    public function __construct()
    {
        parent::__construct('WordPress couldn’t confirm whether a draft saved. Check the same ZIP before importing again.');
    }
}
