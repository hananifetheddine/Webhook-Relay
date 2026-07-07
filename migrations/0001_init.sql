CREATE TABLE users (
    id CHAR(36) NOT NULL,
    email VARCHAR(255) NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE KEY uq_users_email (email)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE endpoints (
    id CHAR(36) NOT NULL,
    user_id CHAR(36) NOT NULL,
    url VARCHAR(2048) NOT NULL,
    event_types JSON NOT NULL,
    secret VARCHAR(128) NOT NULL,
    active TINYINT(1) NOT NULL DEFAULT 1,
    created_at DATETIME(6) NOT NULL,
    PRIMARY KEY (id),
    KEY idx_endpoints_user_active (user_id, active),
    CONSTRAINT fk_endpoints_user FOREIGN KEY (user_id) REFERENCES users (id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE events (
    id CHAR(36) NOT NULL,
    user_id CHAR(36) NOT NULL,
    event_type VARCHAR(255) NOT NULL,
    payload JSON NOT NULL,
    received_at DATETIME(6) NOT NULL,
    PRIMARY KEY (id),
    KEY idx_events_user (user_id),
    CONSTRAINT fk_events_user FOREIGN KEY (user_id) REFERENCES users (id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE delivery_attempts (
    id CHAR(36) NOT NULL,
    event_id CHAR(36) NOT NULL,
    endpoint_id CHAR(36) NOT NULL,
    endpoint_url VARCHAR(2048) NOT NULL,
    status VARCHAR(16) NOT NULL,
    http_status INT NULL,
    attempt_count INT NOT NULL DEFAULT 0,
    next_attempt_at DATETIME(6) NULL,
    last_attempt_at DATETIME(6) NULL,
    created_at DATETIME(6) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE KEY uq_delivery_event_endpoint (event_id, endpoint_id),
    KEY idx_deliveries_due (status, next_attempt_at),
    KEY idx_deliveries_event (event_id),
    CONSTRAINT chk_delivery_status CHECK (status IN ('pending', 'success', 'failed', 'exhausted')),
    CONSTRAINT fk_deliveries_event FOREIGN KEY (event_id) REFERENCES events (id),
    CONSTRAINT fk_deliveries_endpoint FOREIGN KEY (endpoint_id) REFERENCES endpoints (id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
