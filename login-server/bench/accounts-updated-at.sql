-- Track account changes so login-server can reload only new and updated rows.
-- Safe to run more than once.

ALTER TABLE `accounts`
  ADD COLUMN IF NOT EXISTS `updated_at` DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6);

CREATE INDEX IF NOT EXISTS `idx_accounts_updated_at` ON `accounts` (`updated_at`);

CREATE TABLE IF NOT EXISTS `accounts_removed` (
  `account_id` INT NOT NULL,
  `updated_at` DATETIME(6) NOT NULL,
  PRIMARY KEY (`account_id`),
  KEY `idx_accounts_removed_updated_at` (`updated_at`)
) ENGINE=InnoDB;

DROP TRIGGER IF EXISTS `accounts_before_insert_touch`;
CREATE TRIGGER `accounts_before_insert_touch`
BEFORE INSERT ON `accounts`
FOR EACH ROW
SET NEW.`updated_at` = CURRENT_TIMESTAMP(6);

DROP TRIGGER IF EXISTS `accounts_before_update_touch`;
CREATE TRIGGER `accounts_before_update_touch`
BEFORE UPDATE ON `accounts`
FOR EACH ROW
SET NEW.`updated_at` = CURRENT_TIMESTAMP(6);

DROP TRIGGER IF EXISTS `accounts_after_delete_touch`;
CREATE TRIGGER `accounts_after_delete_touch`
AFTER DELETE ON `accounts`
FOR EACH ROW
INSERT INTO `accounts_removed` (`account_id`, `updated_at`)
VALUES (OLD.`id`, CURRENT_TIMESTAMP(6))
ON DUPLICATE KEY UPDATE `updated_at` = CURRENT_TIMESTAMP(6);
