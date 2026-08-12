# DoLogger Compliance Templates

Official compliance configuration templates for common regulatory frameworks.
These templates provide pre-configured settings aligned with regulatory
technical controls, giving your team a head start on compliance readiness.

## Quick Start

Validate a compliance template against your configuration:

```bash
dologctl config validate --config compliance/gdpr.toml --strict
```

Load a template as your configuration base:

```bash
dologctl config apply compliance/hipaa.toml
```

Use with environment variable override:

```bash
DO_LOG_CONFIG_FILE=compliance/pci-dss.toml dologctl config validate --strict
```

## Available Templates

| Template | Regulation | Jurisdiction | Key Controls |
|---|---|---|---|
| `gdpr.toml` | GDPR (Regulation EU 2016/679) | EU/EEA | Data protection, PII handling, records of processing |
| `hipaa.toml` | HIPAA Security Rule (45 CFR Part 164) | United States | ePHI audit controls, integrity controls, transmission security |
| `pci-dss.toml` | PCI DSS v4.0.1 | Global (payment card industry) | Requirement 10 (audit trails), log retention, access monitoring |

## Settings Overview

Each template configures these settings at their most secure values for the
target regulation:

| Setting | Value | Meaning |
|---|---|---|
| `performance_profile` | `"prod-audit"` | Enables signed records, medium batches, audit-optimized batching |
| `level` | `"AUDIT"` | Captures all security-relevant and data-access events |
| `enable_signature` | `true` | Ed25519 cryptographic signatures on every record — non-repudiation |
| `worm_enabled` | `true` | Write-Once-Read-Many storage — prevents log deletion/modification |
| `sign_ring2` | `true` | Includes PII/ePHI fields in signature coverage |
| `escape_html` | `true` | HTML entity escaping on output — prevents log injection attacks |
| `fsync_on_write` | `true` | Forces each write to durable media before acknowledgement |
| `require_tls` | `true` | Requires TLS 1.2+ for all remote sink connections |
| `shutdown_policy` | `"graceful"` | Drains all in-flight records before exit |
| `shutdown_timeout_ms` | `10000` | 10-second drain timeout during shutdown |

### Non-Downgradable Items

All six security booleans (`enable_signature`, `worm_enabled`, `sign_ring2`,
`escape_html`, `fsync_on_write`, `require_tls`) are **non-downgradable items**
per DoLogger design section 2.5.5. This means:

- Child domains can only **tighten** these (false to true), never loosen
- Once a compliance template activates a setting, no configuration source
  can override it to a less-safe value
- Attempting to loosen a non-downgradable item causes initialization failure

## Regulation-Specific Notes

### GDPR (gdpr.toml)

- **Art. 5(1)(f) (Integrity and Confidentiality)**: Cryptographic signatures
  and WORM storage ensure log integrity against unauthorized access
- **Art. 15 (Right of Access)**: Signed Ring 2 fields allow verifiable
  reconstruction of personal data access history for data subject requests
- **Art. 30 (Records of Processing)**: AUDIT-level logging documents all
  processing activities
- **Art. 32 (Security of Processing)**: TLS, fsync, and HTML escaping provide
  multiple layers of technical security measures
- **Art. 35 (DPIA)**: This template supports but does not replace a Data
  Protection Impact Assessment

### HIPAA (hipaa.toml)

- **Section 164.312(b) (Audit Controls)**: AUDIT-level logging captures all
  activity in systems containing ePHI
- **Section 164.312(c)(2) (Integrity Controls)**: Ed25519 signatures and WORM
  storage corroborate that ePHI audit data has not been altered
- **Section 164.312(e)(1) (Transmission Security)**: TLS requirement protects
  ePHI data in transit over networks
- **Section 164.316(b)(2) (Retention)**: WORM storage ensures audit records
  remain intact for the required retention period (typically 6 years)
- **Risk Analysis**: This template is not a replacement for the required
  HIPAA risk analysis (Section 164.308(a)(1)(ii)(A))

### PCI DSS (pci-dss.toml)

- **Requirement 10 (Log and Monitor)**: Full audit trail with non-repudiation
  for all access to cardholder data environments
- **Requirement 10.2**: AUDIT level logs all individual user access to
  cardholder data and administrative actions
- **Requirement 10.3**: Ring 2 signing captures and protects all required
  audit event data elements
- **Requirement 10.5**: Cryptographic signatures and WORM storage protect
  audit trails from unauthorized modification
- **Requirement 10.5.3-10.5.4**: fsync and TLS secure log storage and transit
- **Requirement 10.7 (Retention)**: Retain audit trail history for at least
  12 months, with at least 3 months immediately available

## Verification

After applying a template, verify compliance with:

```bash
# Validate configuration syntax and constraint checks
dologctl config validate --config compliance/gdpr.toml --strict

# Verify log integrity
dologctl verify-log --domain security_audit

# Check non-downgradable items are enforced
dologctl config validate --domain security_audit --check-non-downgradable
```

## Customization

These templates are starting points. You may need to:

1. Adjust `ring_buffer_size` based on your throughput requirements
2. Configure domain-specific settings for different subsystems
3. Add custom sinks (file, syslog, Kafka, OTEL) under `[dologger.sink.*]`
4. Set up domain hierarchy for different security zones

**Changes that weaken security (e.g., setting `enable_signature = false`)
will be rejected by the non-downgradable enforcement system.** You can only
tighten, never loosen.

## Programmatic Validation

The `dologger-core` library provides a `validate_compliance_template()` method
to programmatically verify that a loaded configuration satisfies the minimum
requirements for a given compliance profile:

```rust
use dologger_core::config::{DologgerConfig, ComplianceProfile};

let (config, warnings) = DologgerConfig::load_default();
let profile = ComplianceProfile::Gdpr;
match config.validate_compliance_template(&profile) {
    Ok(()) => println!("Config satisfies {:?} minimum requirements", profile),
    Err(msgs) => {
        for msg in &msgs {
            eprintln!("Compliance gap: {}", msg);
        }
    }
}
```

---

## Legal Disclaimer

**IMPORTANT — READ CAREFULLY BEFORE USE**

The DoLogger project, its maintainers, and contributors **do not provide
legal advice**. The compliance configuration templates provided in this
directory (`gdpr.toml`, `hipaa.toml`, `pci-dss.toml`) and any associated
documentation:

1. **Are technical starting points only.** They represent commonly accepted
   technical configurations aligned with publicly available regulatory
   guidance. They are NOT substitutes for legal analysis.

2. **Do NOT guarantee regulatory compliance.** Achieving and maintaining
   compliance with GDPR, HIPAA, PCI DSS, or any other regulation requires
   a comprehensive organizational program that goes far beyond logging
   configuration — including but not limited to policies, procedures,
   training, risk assessments, breach response planning, and regular audits.

3. **May not address your specific requirements.** Regulatory obligations
   vary by jurisdiction, industry, data types processed, organizational
   structure, and many other factors. Templates based on general best
   practices may not cover obligations specific to your situation.

4. **Require professional review.** The correct configuration for your
   deployment is the sole responsibility of you and your qualified legal
   counsel. You should have all logging and security configurations reviewed
   by a qualified professional familiar with the regulations applicable to
   your organization.

5. **Are not updates.** Regulatory requirements change. It is your
   responsibility to monitor changes to applicable laws and regulations
   and update your configuration accordingly.

**By using these templates, you acknowledge that you are solely responsible
for determining whether your use of DoLogger, including your configuration
choices, satisfies all applicable legal and regulatory obligations.**
