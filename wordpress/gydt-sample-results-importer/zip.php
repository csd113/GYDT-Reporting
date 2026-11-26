<?php

namespace GYDT\SampleResults;

final class Archive
{
    public const MAX_ZIP = 5 * 1024 * 1024;
    public const MAX_PHOTO = 4 * 1024 * 1024;
    private const LIMITS = ['manifest.json' => 8192, 'test_data.json' => 65536];

    private string $bytes;
    private array $members = [];

    public function __construct(string $path, int $limit)
    {
        if (PHP_INT_SIZE !== 8 || $limit < 1) {
            throw new Rejection('The local importer cannot process ZIPs within the configured limit.');
        }
        $limit = min($limit, self::MAX_ZIP);
        $bytes = @file_get_contents($path, false, null, 0, $limit + 1);
        if ($bytes === false || strlen($bytes) < 22 || strlen($bytes) > $limit) {
            throw new Rejection('Choose a supported sample ZIP within the upload limit.');
        }
        $this->bytes = $bytes;
        $this->index();
    }

    public function names(): array
    {
        return array_keys($this->members);
    }

    public function fingerprint(): string
    {
        return hash('sha256', $this->bytes);
    }

    public function read(string $name): string
    {
        $member = $this->members[$name];
        $compressed = $member['compressed'];
        $offset = $member['data_offset'];
        if ($member['method'] === 0) {
            $content = substr($this->bytes, $offset, $compressed);
        } else {
            $stream = inflate_init(ZLIB_ENCODING_RAW);
            $content = '';
            for ($read = 0; $read < $compressed; $read += $length) {
                $length = min(512, $compressed - $read);
                $last = $read + $length === $compressed;
                $part = @inflate_add(
                    $stream,
                    substr($this->bytes, $offset + $read, $length),
                    $last ? ZLIB_FINISH : ZLIB_NO_FLUSH
                );
                if ($part === false || strlen($content) + strlen($part) > $member['size']
                    || (!$last && inflate_get_status($stream) === ZLIB_STREAM_END)) {
                    self::invalid();
                }
                $content .= $part;
            }
            if (inflate_get_status($stream) !== ZLIB_STREAM_END || inflate_get_read_len($stream) !== $compressed) {
                self::invalid();
            }
        }
        if (strlen($content) !== $member['size'] || hash('crc32b', $content) !== sprintf('%08x', $member['crc'])) {
            self::invalid();
        }
        return $content;
    }

    private function index(): void
    {
        [$end, $footer] = $this->directoryFooter();
        $cursor = $footer['offset'];
        $folded = [];
        $total = 0;
        for ($i = 0; $i < $footer['count']; ++$i) {
            if ($cursor + 46 > $end || substr($this->bytes, $cursor, 4) !== "PK\x01\x02") {
                self::invalid();
            }
            $member = $this->header(
                'vmade/vversion/vflags/vmethod/vtime/vdate/Vcrc/Vcompressed/Vsize/vname/vextra/vcomment/vdisk/vinternal/Vattributes/Voffset',
                $cursor + 4,
                42
            );
            $name = substr($this->bytes, $cursor + 46, $member['name']);
            $extra = $cursor + 46 + $member['name'];
            $cursor = $extra + $member['extra'] + $member['comment'];
            $flags = $member['method'] === 8 ? 0x080e : 0x0808;
            if ($cursor > $end || $member['comment'] !== 0 || $member['disk'] !== 0 || $member['offset'] >= $footer['offset']
                || $member['version'] < ($member['method'] === 8 ? 20 : 10) || $member['version'] > 20
                || ($member['flags'] & ~$flags) || !in_array($member['method'], [0, 8], true)
                || $member['compressed'] < 1 || $member['compressed'] > self::MAX_ZIP
                || ($member['method'] === 0 && $member['compressed'] !== $member['size'])
                || !self::regularFileAttributes($member)
                || isset($folded[strtolower($name)])) {
                self::invalid();
            }
            $limit = self::LIMITS[$name] ?? self::MAX_PHOTO;
            if (!isset(self::LIMITS[$name]) && !preg_match('/\A[A-Za-z0-9][A-Za-z0-9 _.-]{0,79}\.(?i:jpe?g)\z/D', $name)) {
                self::invalid();
            }
            if ($member['size'] < 1 || $member['size'] > $limit) {
                throw new Rejection('A ZIP member exceeds its allowed size or is empty.');
            }

            $member['metadata'] = $this->extraFields($extra, $member['extra'], true);
            $folded[strtolower($name)] = true;
            $total += $member['size'];
            $this->members[$name] = $member;
        }
        if ($cursor !== $end || $total > self::MAX_PHOTO + array_sum(self::LIMITS)
            || !isset($this->members['manifest.json'], $this->members['test_data.json'])) {
            self::invalid();
        }

        uasort($this->members, static fn(array $a, array $b): int => $a['offset'] <=> $b['offset']);
        $next = 0;
        $names = $this->names();
        foreach ($names as $index => $name) {
            $boundary = isset($names[$index + 1]) ? $this->members[$names[$index + 1]]['offset'] : $footer['offset'];
            $next = $this->local($name, $next, $boundary);
        }
    }

    private function directoryFooter(): array
    {
        $length = strlen($this->bytes);
        $scan = max(0, $length - 22 - 65535);
        $match = null;
        while (($end = strpos($this->bytes, "PK\x05\x06", $scan)) !== false && $end + 22 <= $length) {
            $scan = $end + 1;
            $footer = $this->header('vdisk/vstart/vlocal/vcount/Vsize/Voffset/vcomment', $end + 4, 18);
            if ($end + 22 + $footer['comment'] !== $length || $footer['disk'] !== 0 || $footer['start'] !== 0
                || $footer['local'] !== $footer['count'] || !in_array($footer['count'], [2, 3], true)
                || $footer['offset'] + $footer['size'] !== $end) {
                continue;
            }
            if ($match !== null) {
                self::invalid();
            }
            $match = [$end, $footer];
        }
        if ($match === null) {
            self::invalid();
        }
        return $match;
    }

    private static function regularFileAttributes(array $member): bool
    {
        $platform = $member['made'] >> 8;
        $dos = $member['attributes'] & 0xffff;
        $unix = $member['attributes'] >> 16;
        // DOS/NTFS share the permitted regular-file flags. Unix may also supply mode bits.
        return in_array($platform, [0, 3, 10], true)
            && ($member['internal'] & ~1) === 0
            && ($dos & ~0xa1) === 0
            && (($dos & 0x80) === 0 || $dos === 0x80)
            && ($platform === 3 || $unix === 0)
            && in_array($unix & ~0777, [0, 0100000], true);
    }

    private function local(string $name, int $offset, int $boundary): int
    {
        $member = $this->members[$name];
        if ($member['offset'] !== $offset || $offset + 30 > $boundary
            || substr($this->bytes, $offset, 4) !== "PK\x03\x04") {
            self::invalid();
        }
        $local = $this->header('vversion/vflags/vmethod/vtime/vdate/Vcrc/Vcompressed/Vsize/vname/vextra', $offset + 4, 26);
        $extra = $offset + 30 + $local['name'];
        $data = $extra + $local['extra'];
        $end = $data + $member['compressed'];
        if ($end > $boundary || $local['name'] !== strlen($name)
            || substr($this->bytes, $offset + 30, $local['name']) !== $name) {
            self::invalid();
        }
        $metadata = $this->extraFields($extra, $local['extra'], false);
        foreach (array_intersect_key($member['metadata'], $metadata) as $id => $value) {
            if ($value !== $metadata[$id]) {
                self::invalid();
            }
        }
        foreach (['version', 'flags', 'method', 'time', 'date'] as $key) {
            if ($local[$key] !== $member[$key]) {
                self::invalid();
            }
        }
        $descriptor = ($member['flags'] & 8) !== 0;
        foreach (['crc', 'compressed', 'size'] as $key) {
            if ($local[$key] !== $member[$key] && !($descriptor && $local[$key] === 0)) {
                self::invalid();
            }
        }
        if ($descriptor) {
            if ($boundary - $end === 16 && substr($this->bytes, $end, 4) === "PK\x07\x08") {
                $end += 4;
            }
            if ($boundary - $end !== 12) {
                self::invalid();
            }
            foreach ($this->header('Vcrc/Vcompressed/Vsize', $end, 12) as $key => $value) {
                if ($value !== $member[$key]) {
                    self::invalid();
                }
            }
            $end += 12;
        }
        if ($end !== $boundary) {
            self::invalid();
        }
        $this->members[$name]['data_offset'] = $data;
        return $end;
    }

    private function extraFields(int $offset, int $length, bool $central): array
    {
        $end = $offset + $length;
        $fields = [];
        while ($offset < $end) {
            if ($end - $offset < 4) {
                self::invalid();
            }
            ['id' => $id, 'length' => $size] = $this->header('vid/vlength', $offset, 4);
            $offset += 4;
            if ($offset + $size > $end || array_key_exists($id, $fields)) {
                self::invalid();
            }
            $data = substr($this->bytes, $offset, $size);
            $fields[$id] = match ($id) {
                0x5455 => self::extendedTimestamp($data, $central),
                0x7875 => self::unixOwner($data),
                0x000a => self::ntfsTimes($data),
                default => self::invalid(),
            };
            $offset += $size;
        }
        return $fields;
    }

    private static function extendedTimestamp(string $data, bool $central): string
    {
        $flags = isset($data[0]) ? ord($data[0]) : 0;
        $stored = $central ? $flags & 1 : $flags;
        if (($flags & ~7) || strlen($data) !== 1 + 4 * substr_count(decbin($stored), '1')) {
            self::invalid();
        }
        // Only the mtime can appear in both headers. Optional atime and ctime remain local.
        return ($flags & 1) ? substr($data, 1, 4) : '';
    }

    private static function unixOwner(string $data): string
    {
        $uid = isset($data[1]) ? ord($data[1]) : 0;
        $gid = isset($data[2 + $uid]) ? ord($data[2 + $uid]) : 0;
        if (($data[0] ?? '') !== "\x01" || $uid < 1 || $uid > 8 || $gid < 1 || $gid > 8
            || strlen($data) !== 3 + $uid + $gid) {
            self::invalid();
        }
        return $data;
    }

    private static function ntfsTimes(string $data): string
    {
        if (strlen($data) !== 32 || substr($data, 0, 8) !== "\0\0\0\0\x01\0\x18\0") {
            self::invalid();
        }
        return $data;
    }

    private function header(string $format, int $offset, int $length): array
    {
        if ($offset < 0 || $offset + $length > strlen($this->bytes)) {
            self::invalid();
        }
        return unpack($format, $this->bytes, $offset);
    }

    private static function invalid(): never
    {
        throw new Rejection('Choose a supported sample ZIP with valid, matching file headers and contents.');
    }
}
