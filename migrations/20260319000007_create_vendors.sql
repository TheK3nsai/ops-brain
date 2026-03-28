CREATE TABLE vendors (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT,
    account_number TEXT,
    support_phone TEXT,
    support_email TEXT,
    support_portal TEXT,
    sla_summary TEXT,
    contract_end DATE,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE vendor_clients (
    vendor_id UUID NOT NULL REFERENCES vendors(id) ON DELETE CASCADE,
    client_id UUID NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    PRIMARY KEY (vendor_id, client_id)
);
