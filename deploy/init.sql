-- Taoli Trading System Database Initialization

-- Create extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Create custom types
CREATE TYPE order_side AS ENUM ('buy', 'sell');
CREATE TYPE order_status AS ENUM ('pending', 'submitted', 'open', 'partial', 'filled', 'cancelled', 'rejected', 'expired', 'unknown');
CREATE TYPE venue AS ENUM ('binance', 'bybit');
CREATE TYPE alert_level AS ENUM ('critical', 'high', 'medium', 'info');

-- Create tables
CREATE TABLE IF NOT EXISTS instruments (
    id SERIAL PRIMARY KEY,
    venue venue NOT NULL,
    symbol VARCHAR(50) NOT NULL,
    base_asset VARCHAR(20) NOT NULL,
    quote_asset VARCHAR(20) NOT NULL,
    step_size DECIMAL(20,8) NOT NULL,
    min_qty DECIMAL(20,8) NOT NULL,
    max_qty DECIMAL(20,8) NOT NULL,
    tick_size DECIMAL(20,8) NOT NULL,
    min_notional DECIMAL(20,8) NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(venue, symbol)
);

CREATE TABLE IF NOT EXISTS orders (
    id SERIAL PRIMARY KEY,
    venue venue NOT NULL,
    external_order_id VARCHAR(100),
    side order_side NOT NULL,
    symbol VARCHAR(50) NOT NULL,
    quantity DECIMAL(20,8) NOT NULL,
    price DECIMAL(20,8),
    status order_status NOT NULL DEFAULT 'pending',
    filled_quantity DECIMAL(20,8) DEFAULT 0,
    average_price DECIMAL(20,8),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    submitted_at TIMESTAMP WITH TIME ZONE,
    completed_at TIMESTAMP WITH TIME ZONE
);

CREATE TABLE IF NOT EXISTS trades (
    id SERIAL PRIMARY KEY,
    venue venue NOT NULL,
    external_trade_id VARCHAR(100) NOT NULL,
    order_id INTEGER REFERENCES orders(id),
    side order_side NOT NULL,
    symbol VARCHAR(50) NOT NULL,
    quantity DECIMAL(20,8) NOT NULL,
    price DECIMAL(20,8) NOT NULL,
    fee DECIMAL(20,8) DEFAULT 0,
    fee_asset VARCHAR(20),
    executed_at TIMESTAMP WITH TIME ZONE NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(venue, external_trade_id)
);

CREATE TABLE IF NOT EXISTS balances (
    id SERIAL PRIMARY KEY,
    venue venue NOT NULL,
    asset VARCHAR(20) NOT NULL,
    available DECIMAL(20,8) NOT NULL DEFAULT 0,
    locked DECIMAL(20,8) NOT NULL DEFAULT 0,
    total DECIMAL(20,8) NOT NULL DEFAULT 0,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(venue, asset)
);

CREATE TABLE IF NOT EXISTS alerts (
    id SERIAL PRIMARY KEY,
    alert_id VARCHAR(100) NOT NULL UNIQUE,
    level alert_level NOT NULL,
    alert_type VARCHAR(50) NOT NULL,
    message TEXT NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'pending',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE,
    acknowledged_at TIMESTAMP WITH TIME ZONE,
    resolved_at TIMESTAMP WITH TIME ZONE
);

CREATE TABLE IF NOT EXISTS audit_log (
    id SERIAL PRIMARY KEY,
    event_type VARCHAR(50) NOT NULL,
    entity_type VARCHAR(50) NOT NULL,
    entity_id INTEGER,
    old_value JSONB,
    new_value JSONB,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Create indexes
CREATE INDEX IF NOT EXISTS idx_orders_venue_symbol ON orders(venue, symbol);
CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
CREATE INDEX IF NOT EXISTS idx_trades_venue_symbol ON trades(venue, symbol);
CREATE INDEX IF NOT EXISTS idx_trades_executed_at ON trades(executed_at);
CREATE INDEX IF NOT EXISTS idx_alerts_status ON alerts(status);
CREATE INDEX IF NOT EXISTS idx_alerts_level ON alerts(level);
CREATE INDEX IF NOT EXISTS idx_audit_log_entity ON audit_log(entity_type, entity_id);

-- Create functions
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

-- Create triggers
CREATE TRIGGER update_instruments_updated_at BEFORE UPDATE ON instruments
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_orders_updated_at BEFORE UPDATE ON orders
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_balances_updated_at BEFORE UPDATE ON balances
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Insert default instruments
INSERT INTO instruments (venue, symbol, base_asset, quote_asset, step_size, min_qty, max_qty, tick_size, min_notional)
VALUES
    ('binance', 'BTCUSDT', 'BTC', 'USDT', 0.00001, 0.00001, 9000, 0.01, 10),
    ('binance', 'ETHUSDT', 'ETH', 'USDT', 0.0001, 0.0001, 9000, 0.01, 10),
    ('bybit', 'BTCUSDT', 'BTC', 'USDT', 0.00001, 0.00001, 9000, 0.01, 10),
    ('bybit', 'ETHUSDT', 'ETH', 'USDT', 0.0001, 0.0001, 9000, 0.01, 10)
ON CONFLICT (venue, symbol) DO NOTHING;
