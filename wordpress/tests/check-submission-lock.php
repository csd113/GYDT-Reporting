<?php

namespace GYDT\SampleResults {
    // Keep the preflight and advisory operations on the driver boundary in the test as well.
    function mysqli_query(\mysqli $connection, string $sql): object|bool
    {
        global $wpdb;
        if ($connection !== $wpdb->dbh) {
            return false;
        }
        if ($sql === 'START TRANSACTION') {
            return $wpdb->query($sql) !== false;
        }
        return new class($wpdb->get_var($sql)) {
            public function __construct(private ?string $value) {}
            public function fetch_row(): array { return [$this->value]; }
            public function free(): void {}
        };
    }
}

namespace {

use GYDT\SampleResults\SubmissionLock;

if (PHP_SAPI !== 'cli' || $argc !== 2) {
    exit(1);
}

require __DIR__ . '/../gydt-sample-results-importer/submission-lock.php';
define('DB_NAME', 'synthetic_lock_contract');

final class LockDatabase
{
    public ?mysqli $dbh;
    public string $posts = 'synthetic_posts';
    public bool $ready = true;
    public int $reconnect_retries = 5;
    public bool $inTransaction = false;
    public bool $held = false;
    public bool $timeout = false;
    public bool $stateUnknown = false;
    public bool $rollbackError = false;
    public int $releaseCount = 0;
    public int $closeCount = 0;
    public int $acquireCount = 0;

    public function __construct()
    {
        $this->dbh = mysqli_init();
    }

    public function prepare(string $sql, string $name): string
    {
        return sprintf($sql, $name);
    }

    public function get_var(string $sql): ?string
    {
        if (str_starts_with($sql, 'SELECT GET_LOCK')) {
            ++$this->acquireCount;
            $this->held = !$this->timeout;
            return $this->held ? '1' : '0';
        }
        if (str_starts_with($sql, 'SELECT IS_USED_LOCK')) {
            return $this->held ? '1' : '0';
        }
        if ($sql === 'SELECT @@in_transaction = 0 AND @@autocommit = 1') {
            return $this->stateUnknown ? null : ($this->inTransaction ? '0' : '1');
        }
        if ($sql === 'SELECT @@in_transaction') {
            return $this->inTransaction ? '1' : '0';
        }
        if (str_starts_with($sql, 'SELECT RELEASE_LOCK')) {
            ++$this->releaseCount;
            $this->held = false;
            return '1';
        }
        throw new RuntimeException('Unexpected query');
    }

    public function query(string $sql): int|false
    {
        if ($sql === 'START TRANSACTION') {
            $this->inTransaction = true;
            return 0;
        }
        if ($sql !== 'ROLLBACK') {
            throw new RuntimeException('Unexpected command');
        }
        if ($this->rollbackError) {
            return false;
        }
        $this->inTransaction = false;
        return 0;
    }

    public function close(): bool
    {
        ++$this->closeCount;
        $this->held = false;
        $this->inTransaction = false;
        $this->dbh = null;
        return true;
    }
}

$wpdb = new LockDatabase();
$case = $argv[1];
if (in_array($case, ['timeout', 'ambient'], true)) {
    $wpdb->timeout = $case === 'timeout';
    $wpdb->inTransaction = $case === 'ambient';
    if (SubmissionLock::acquire() || $wpdb->held || $wpdb->reconnect_retries !== 5 || $wpdb->releaseCount !== 0
        || $wpdb->closeCount !== 0 || $wpdb->inTransaction !== ($case === 'ambient')
        || $wpdb->acquireCount !== ($case === 'timeout' ? 1 : 0)) {
        throw new RuntimeException('A timed-out lock must not authorize a write or release a lock it did not obtain.');
    }
} elseif ($case === 'external-after-lock') {
    if (!SubmissionLock::acquire()) {
        throw new RuntimeException('Could not acquire the test lock.');
    }
    $wpdb->inTransaction = true;
    SubmissionLock::release();
    if (!$wpdb->inTransaction || $wpdb->releaseCount !== 0 || $wpdb->closeCount !== 0 || SubmissionLock::acquire()) {
        throw new RuntimeException('The plugin must leave an unowned transaction alone.');
    }
} else {
    if (!SubmissionLock::acquire() || !SubmissionLock::begin()) {
        throw new RuntimeException('Could not acquire the test lock.');
    }
    $wpdb->inTransaction = true;
    $wpdb->stateUnknown = $case === 'state-unknown';
    $wpdb->rollbackError = $case === 'rollback-failed';
    if (!in_array($case, ['clear', 'state-unknown', 'rollback-failed', 'dirty-release'], true)) {
        throw new RuntimeException('Unsupported test case.');
    }
    if ($case !== 'dirty-release' && SubmissionLock::rollback() !== ($case === 'clear')) {
        throw new RuntimeException('The rollback did not match the observable transaction state.');
    }
    SubmissionLock::release();
    if (in_array($case, ['clear', 'dirty-release'], true)) {
        if ($wpdb->releaseCount !== 1 || !$wpdb->ready || $wpdb->closeCount !== 0 || $wpdb->reconnect_retries !== 5) {
            throw new RuntimeException('A verified clean connection should release and recover its normal settings.');
        }
    } elseif ($wpdb->ready || $wpdb->releaseCount !== 0 || $wpdb->closeCount !== 1 || $wpdb->reconnect_retries !== 0
        || SubmissionLock::acquire()) {
        throw new RuntimeException('A dirty or uncertain connection must be discarded without releasing the lock through SQL.');
    }
}
echo "Submission lock contract passed for $case.\n";
}
