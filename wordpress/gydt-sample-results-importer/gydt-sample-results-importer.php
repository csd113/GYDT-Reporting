<?php
/**
 * Plugin Name: GYDT Sample Results Importer
 * Description: Synthetic fixture importer for disposable local testing only.
 * Version: 0.1.1-fixture.1
 * Requires PHP: 8.1
 */

namespace GYDT\SampleResults;

if (!defined('ABSPATH')) {
    exit;
}

require_once __DIR__ . '/pilot.php';

// The example never registers hooks outside an explicitly enabled local fixture.
if (!Pilot::local()) {
    return;
}

require_once __DIR__ . '/contract.php';
require_once __DIR__ . '/zip.php';
require_once __DIR__ . '/submission-lock.php';
require_once __DIR__ . '/acf-contract.php';
require_once __DIR__ . '/import-history.php';
require_once __DIR__ . '/import.php';

const IMPORT_ACTION = 'gydt_import_sample_results';

add_action('admin_menu', static function (): void {
    if (Import::allowed()) {
        $page = add_submenu_page(
            'edit.php?post_type=results',
            'Import Sample Results',
            'Import Sample Results',
            get_post_type_object('results')->cap->create_posts,
            IMPORT_ACTION,
            __NAMESPACE__ . '\\renderPage'
        );
        if ($page !== false) {
            add_action('load-' . $page, static function (): void {
                wp_enqueue_script(IMPORT_ACTION, plugins_url('queue.js', __FILE__), [], (string) filemtime(__DIR__ . '/queue.js'), true);
                wp_enqueue_style(IMPORT_ACTION, plugins_url('queue.css', __FILE__), [], (string) filemtime(__DIR__ . '/queue.css'));
            });
        }
    }
});
add_action('admin_post_' . IMPORT_ACTION, __NAMESPACE__ . '\\handleUpload');
add_action('admin_post_' . ImportHistory::ACTION, [ImportHistory::class, 'handle']);
add_filter('wp_insert_post_empty_content', [Import::class, 'allowSavedZero'], PHP_INT_MAX, 2);

function handleUpload(): never
{
    nocache_headers();
    if (!Import::allowed() || ($_SERVER['REQUEST_METHOD'] ?? '') !== 'POST') {
        wp_send_json(['state' => 'failed', 'message' => 'This request can’t import Test Results. Return to Import Sample Results and try again.'], 403);
    }
    $postLimit = wp_convert_hr_to_bytes(ini_get('post_max_size'));
    if ($postLimit > 0 && (int) ($_SERVER['CONTENT_LENGTH'] ?? 0) > $postLimit) {
        wp_send_json(['state' => 'failed', 'message' => 'This ZIP exceeds the site’s upload limit. No draft was saved.'], 413);
    }
    if (!verifiedNonce($_POST['_wpnonce'] ?? null, IMPORT_ACTION)) {
        wp_send_json(['state' => 'failed', 'message' => 'This request expired. Reload Import Sample Results and try again.'], 403);
    }

    $file = $_FILES['sample_zip'] ?? null;
    $mode = $_POST['mode'] ?? null;
    $writing = false;
    try {
        if (!in_array($mode, ['preview', 'import'], true) || count($_FILES) !== 1 || !is_array($file)
            || ($file['error'] ?? null) !== UPLOAD_ERR_OK || !is_string($file['tmp_name'] ?? null)
            || !is_uploaded_file($file['tmp_name'])) {
            throw new Rejection('No draft was saved. Choose one supported ZIP within the upload limit and try again.');
        }
        $package = Package::load($file['tmp_name'], wp_max_upload_size());
        // The authenticated page nonce authorizes a draft import. Each request
        // independently validates its uploaded archive before any writes.
        $writing = $mode === 'import';
        [$state, $postId] = Import::run($package, $mode === 'preview');
    } catch (UnknownOutcome $exception) {
        $response = ['state' => 'unknown', 'message' => $writing
            ? $exception->getMessage()
            : 'The check didn’t finish. No import was attempted.'];
    } catch (Conflict $exception) {
        $response = ['state' => 'conflict', 'message' => $exception->getMessage()];
    } catch (Rejection $exception) {
        $response = ['state' => 'failed', 'message' => $exception->getMessage()];
    } catch (\Throwable) {
        $response = $writing
            ? ['state' => 'unknown', 'message' => 'WordPress couldn’t confirm whether a draft saved.']
            : ['state' => 'failed', 'message' => 'WordPress couldn’t check this ZIP. No import was attempted.'];
    }
    if (isset($state)) {
        $response = [
            'state' => $state,
            'message' => match ($state) {
                'ready' => 'Ready to import.',
                'duplicate' => 'This sample was already imported.',
                'imported' => 'Draft saved. Open Review draft to check it.',
                'trash' => 'No new draft was saved. The matching Result is in Test Results → Trash.',
            },
            'sample' => sampleIdentity($package),
        ];
    }
    if (isset($postId, $state) && in_array($state, ['imported', 'duplicate'], true)) {
        try {
            $response['result'] = ImportHistory::receipt(get_post($postId));
        } catch (\Throwable) {
            // A link filter cannot change the confirmed database outcome.
        }
    }
    wp_send_json($response);
}

function sampleIdentity(Package $package): array
{
    $title = $package->data['submission_number'];
    // Match the importer's standard title format for display and preserve leading zeroes.
    $standard = preg_match('/\A[0-9]+-[0-9]{3}-([0-9]{4}-[0-9]{2}-[0-9]{2})-([0-9]{3,})\z/D', $title, $parts)
        && Package::validDate($parts[1]) && ltrim($parts[2], '0') !== '';
    return [
        'submission_number' => $title,
        'analysis_date' => $package->data['analysis_date'],
        'sample_number' => $standard ? $parts[2] : null,
    ];
}

function verifiedNonce(mixed $nonce, string $action): bool
{
    return is_string($nonce) && wp_verify_nonce($nonce, $action) !== false;
}

function positiveInteger(mixed $raw): int
{
    return is_string($raw) && preg_match('/\A[1-9][0-9]*\z/D', $raw) && (string) (int) $raw === $raw ? (int) $raw : 0;
}

function renderPage(): void
{
    if (!Import::allowed()) {
        wp_die('Import Sample Results isn’t available to this account or site. Ask an administrator.', '', ['response' => 403]);
    }
    ?>
    <div class="wrap">
        <h1>Import Sample Results</h1>
        <form id="gydt-import-queue" action="<?php echo esc_url(add_query_arg('action', IMPORT_ACTION, admin_url('admin-post.php'))); ?>" method="post" enctype="multipart/form-data">
            <?php wp_nonce_field(IMPORT_ACTION, '_wpnonce', false); ?>
            <p id="gydt-upload-help">Choose or drop up to 20 sample ZIPs. Each successful import saves a draft. Sample photos go into the WordPress Media Library and can be viewed by direct link even while the Result is a draft. Review drafts in Test Results before publishing.</p>
            <div id="gydt-zip-drop" role="group" aria-labelledby="gydt-drop-label" aria-describedby="gydt-upload-help">
                <strong id="gydt-drop-label">Drop sample ZIPs here</strong>
            </div>
            <p><label for="gydt-sample-zip">Choose sample ZIPs</label>
                <input type="file" name="sample_zip" id="gydt-sample-zip" accept=".zip" multiple aria-describedby="gydt-upload-help"></p>
            <p><label for="gydt-check-zip">Check ZIPs without importing</label>
                <input type="file" id="gydt-check-zip" accept=".zip" multiple aria-describedby="gydt-check-help"></p>
            <p id="gydt-check-help" class="description">Choose the same ZIP to see whether a draft already exists. This won’t import it.</p>
            <p id="gydt-queue-message" role="status"></p>
            <ul id="gydt-queue-items" aria-label="Sample ZIP queue"></ul>
            <noscript><p>Enable JavaScript to upload sample ZIPs.</p></noscript>
        </form>
        <section id="gydt-history" aria-labelledby="gydt-history-heading"
            data-url="<?php echo esc_url(add_query_arg('action', ImportHistory::ACTION, admin_url('admin-post.php'))); ?>"
            data-nonce="<?php echo esc_attr(wp_create_nonce(ImportHistory::ACTION)); ?>">
            <h2 id="gydt-history-heading">Today’s uploads</h2>
            <p class="description">Shows your uploads for today in Vancouver. Hiding an upload doesn’t change its Test Result.</p>
            <p class="gydt-history-controls">
                <button type="button" class="button" id="gydt-history-refresh">Refresh status</button>
                <button type="button" class="button" id="gydt-history-clear" disabled>Clear list</button>
            </p>
            <p id="gydt-history-message" role="status" tabindex="-1"></p>
            <ul id="gydt-history-items" aria-label="Your uploads today"></ul>
            <p class="gydt-history-controls">
                <button type="button" class="button" id="gydt-history-newest" hidden>Newest uploads</button>
                <button type="button" class="button" id="gydt-history-older" hidden>Older uploads</button>
            </p>
        </section>
    </div>
    <?php
}
