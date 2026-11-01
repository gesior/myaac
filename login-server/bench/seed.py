#!/usr/bin/env python3
"""Insert bench accounts and three characters each. Pipe into the mysql client."""

import sys

PASSWORD_SHA1 = "e5e9fa1ba31ecd1ae84f75caaa474f3a663f05f4"
BATCH = 400


def chunks(start, end):
    cursor = start
    while cursor <= end:
        stop = min(cursor + BATCH - 1, end)
        yield cursor, stop
        cursor = stop + 1


def main():
    accounts = int(sys.argv[1]) if len(sys.argv) > 1 else 10000
    if accounts < 1 or accounts > 99999:
        raise SystemExit("account count must be 1..99999")

    print("SET FOREIGN_KEY_CHECKS=0;")
    print("TRUNCATE TABLE `players`;")
    print("TRUNCATE TABLE `accounts`;")
    print("SET FOREIGN_KEY_CHECKS=1;")

    for start, stop in chunks(1, accounts):
        rows = []
        for i in range(start, stop + 1):
            rows.append(
                "('bench{0:05d}','{1}','bench{0:05d}@bench.test',0,UNIX_TIMESTAMP())".format(
                    i, PASSWORD_SHA1
                )
            )
        print(
            "INSERT INTO `accounts` (`name`,`password`,`email`,`premium_ends_at`,`creation`) VALUES"
        )
        print(",\n".join(rows) + ";")

    # Ids are 1..N after TRUNCATE. Three characters, highest experience is the main one.
    characters = (
        ("a", 8, 1, 1, 100, 136),
        ("b", 20, 0, 2, 1500, 128),
        ("c", 40, 1, 4, 9000, 131),
    )
    for suffix, level, sex, vocation, experience, look in characters:
        for start, stop in chunks(1, accounts):
            rows = []
            for i in range(start, stop + 1):
                rows.append(
                    "('p{0:05d}{1}',{0},{2},{3},{4},{5},{6},0)".format(
                        i, suffix, level, sex, vocation, experience, look
                    )
                )
            print(
                "INSERT INTO `players` (`name`,`account_id`,`level`,`sex`,`vocation`,`experience`,`looktype`,`deletion`) VALUES"
            )
            print(",\n".join(rows) + ";")

    print("SELECT COUNT(*) AS accounts FROM `accounts`;")
    print("SELECT COUNT(*) AS players FROM `players`;")


if __name__ == "__main__":
    main()
