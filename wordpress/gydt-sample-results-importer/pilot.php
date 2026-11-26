<?php

namespace GYDT\SampleResults;

final class Pilot
{
    public static function local(): bool
    {
        return wp_get_environment_type() === 'local'
            && defined('GYDT_ENABLE_SYNTHETIC_FIXTURE') && GYDT_ENABLE_SYNTHETIC_FIXTURE === true;
    }
}
