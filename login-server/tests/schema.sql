-- Fixture for docker compose. Not the production schema.
CREATE TABLE IF NOT EXISTS `accounts` (
  `id` INT NOT NULL AUTO_INCREMENT,
  `name` VARCHAR(32) NOT NULL,
  `email` VARCHAR(255) NOT NULL,
  `password` CHAR(40) NOT NULL,
  `secret` VARCHAR(64) NULL,
  `email_verified` TINYINT NOT NULL DEFAULT 0,
  `premdays` INT NOT NULL DEFAULT 0,
  `lastday` INT UNSIGNED NOT NULL DEFAULT 0,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS `players` (
  `id` INT NOT NULL AUTO_INCREMENT,
  `name` VARCHAR(255) NOT NULL,
  `account_id` INT NOT NULL,
  `level` INT NOT NULL DEFAULT 1,
  `sex` INT NOT NULL DEFAULT 0,
  `vocation` INT NOT NULL DEFAULT 0,
  `promotion` INT NOT NULL DEFAULT 0,
  `experience` BIGINT NOT NULL DEFAULT 0,
  `looktype` INT NOT NULL DEFAULT 128,
  `lookhead` INT NOT NULL DEFAULT 0,
  `lookbody` INT NOT NULL DEFAULT 0,
  `looklegs` INT NOT NULL DEFAULT 0,
  `lookfeet` INT NOT NULL DEFAULT 0,
  `lookaddons` INT NOT NULL DEFAULT 0,
  `deleted` TINYINT NOT NULL DEFAULT 0,
  PRIMARY KEY (`id`),
  KEY `account_id` (`account_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- sha1("secret") = e5e9fa1ba31ecd1ae84f75caaa474f3a663f05f4
INSERT INTO `accounts` (`id`, `name`, `email`, `password`, `secret`, `email_verified`, `premdays`, `lastday`) VALUES
(1, 'tester', 'tester@example.com', 'e5e9fa1ba31ecd1ae84f75caaa474f3a663f05f4', NULL, 1, 0, 0),
(2, 'hero', 'hero@example.com', 'e5e9fa1ba31ecd1ae84f75caaa474f3a663f05f4', NULL, 1, 10, UNIX_TIMESTAMP()),
(3, 'otpuser', 'otp@example.com', 'e5e9fa1ba31ecd1ae84f75caaa474f3a663f05f4', 'JBSWY3DPEHPK3PXP', 1, 0, 0);

INSERT INTO `players` (`id`, `name`, `account_id`, `level`, `sex`, `vocation`, `promotion`, `experience`, `looktype`, `lookhead`, `lookbody`, `looklegs`, `lookfeet`, `lookaddons`, `deleted`) VALUES
(1, 'High', 2, 50, 1, 4, 1, 500000, 128, 78, 69, 58, 76, 0, 0),
(2, 'Low', 2, 8, 0, 1, 0, 100, 136, 0, 0, 0, 0, 0, 0),
(3, 'Ghost', 2, 1, 1, 0, 0, 1, 128, 0, 0, 0, 0, 0, 1);
