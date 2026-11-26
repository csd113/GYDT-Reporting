<?php

namespace GYDT\SampleResults;

final class SubmissionLock
{
    private static int $depth = 0;
    private static int $retries = 0;
    private static string $heldName = '';
    private static ?\mysqli $connection = null;
    private static bool $transaction = false;
    private static bool $discarded = false;
    public static function acquire(): bool
    {
        global $wpdb;
        if (self::$discarded || !$wpdb->dbh instanceof \mysqli) {
            return false;
        }
        if (self::$depth) {
            if (!self::held()) {
                return false;
            }
            ++self::$depth;
            return true;
        }
        $connection = $wpdb->dbh;
        if (!self::cleanLink($connection) || !self::lockLink($connection)) {
            return false;
        }
        $retries = $wpdb->reconnect_retries;
        $name = self::name();
        // A retry on a new DB connection could write without the named lock or transaction.
        $wpdb->reconnect_retries = 0;
        self::$retries = $retries;
        self::$heldName = $name;
        self::$connection = $connection;
        self::$depth = 1;
        return true;
    }

    private static function cleanLink(\mysqli $connection): bool
    {
        // Use the existing driver link so an ambient transaction cannot trigger wpdb retry or query hooks.
        return self::linkValue($connection, 'SELECT @@in_transaction = 0 AND @@autocommit = 1') === '1';
    }

    private static function lockLink(\mysqli $connection): bool
    {
        $name = self::name(); // Fixed prefix and hex digest only.
        return self::linkValue($connection, "SELECT GET_LOCK('$name', 10)") === '1';
    }

    private static function linkValue(\mysqli $connection, string $sql): ?string
    {
        try {
            $result = mysqli_query($connection, $sql);
            if ($result === false) {
                return null;
            }
            $row = $result->fetch_row();
            $result->free();
            return isset($row[0]) ? (string) $row[0] : null;
        } catch (\Throwable) {
            return null;
        }
    }

    public static function begin(): bool
    {
        if (self::$transaction || !self::inAutocommit()) {
            return false;
        }
        self::$transaction = true;
        try {
            return mysqli_query(self::$connection, 'START TRANSACTION') !== false && self::inTransaction();
        } catch (\Throwable) {
            return false;
        }
    }

    public static function inTransaction(): bool
    {
        return self::$transaction && self::held() && self::linkValue(self::$connection, 'SELECT @@in_transaction') === '1';
    }

    public static function commit(): bool
    {
        global $wpdb;
        if (!self::inTransaction() || $wpdb->query('COMMIT') === false || !self::inAutocommit()) {
            return false;
        }
        self::$transaction = false;
        return true;
    }

    public static function held(): bool
    {
        global $wpdb;
        $name = self::$heldName;
        return self::$depth > 0 && self::$connection === $wpdb->dbh && self::name() === $name
            && self::linkValue(self::$connection, "SELECT IS_USED_LOCK('$name') = CONNECTION_ID()") === '1';
    }

    public static function inAutocommit(): bool
    {
        return self::held() && self::cleanLink(self::$connection);
    }

    public static function release(): void
    {
        global $wpdb;
        if (self::$depth > 1) {
            --self::$depth;
        } elseif (self::$depth === 1) {
            if (self::$transaction && !self::rollback()) {
                return;
            }
            if (self::inAutocommit()) {
                self::linkValue(self::$connection, "SELECT RELEASE_LOCK('" . self::$heldName . "')");
            }
            self::$depth = 0;
            self::$heldName = '';
            self::$connection = null;
            $wpdb->reconnect_retries = self::$retries;
        }
    }

    public static function rollback(): bool
    {
        global $wpdb;
        if (!self::$transaction) {
            return self::inAutocommit();
        }
        if (self::held() && $wpdb->query('ROLLBACK') !== false && self::inAutocommit()) {
            self::$transaction = false;
            return true;
        }
        self::discard();
        return false;
    }

    private static function discard(): void
    {
        global $wpdb;
        $connection = self::$connection;
        self::$discarded = true;
        self::$transaction = false;
        self::$connection = null;
        self::$heldName = '';
        self::$depth = 0;
        // Stop all further wpdb work in this request, including WordPress shutdown hooks.
        $wpdb->reconnect_retries = 0;
        $wpdb->ready = false;
        try {
            if ($connection && ($connection !== $wpdb->dbh || !$wpdb->close())) {
                mysqli_close($connection);
            }
        } catch (\Throwable) {
            // The link may already be closed. Do not release the named lock via SQL.
        }
    }

    private static function name(): string
    {
        global $wpdb;
        return 'gydt-import:' . substr(hash('sha256', DB_NAME . ':' . $wpdb->posts), 0, 48);
    }
}
