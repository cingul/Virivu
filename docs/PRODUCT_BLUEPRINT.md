# Virivu Product Blueprint

## Product Name

Primary name: **Virivu Research Cloud**  
Optional alternatives:
- TrialMesh
- Cliniflow OS
- MosaicTrials

## Vision

Build an all-in-one research operating system that makes study execution faster, safer, and more patient-friendly than legacy tools (including REDCap-like point solutions).

## Primary Users

- Principal investigators
- Study coordinators and site managers
- Sponsors (device/pharma)
- Monitors and collaborators
- Patients and caregivers
- Biostatistics and data science teams

## Core Platform Modules

1. **Tenant & Study Operations**
   - Multi-organization and multi-site tenancy
   - Study/trial setup templates
   - Site-level dashboards
   - Role-based permissions

2. **eSource & eCRF / Forms**
   - Form builder (demographics, intake, outcomes, adverse events)
   - Branching logic and versioning
   - eConsent workflows and signature capture
   - Form packets scheduled by visit or event

3. **Patient Engagement**
   - Secure patient portal
   - SMS/email/app-based form invites
   - Home video/photo capture uploads
   - Multilingual and accessibility support

4. **Media & Evidence Capture**
   - Device camera upload
   - Time- and user-stamped metadata
   - Optional clinician review queue
   - Redaction and retention controls

5. **AI Assistance**
   - Guided patient instructions by protocol
   - Clinical conversation capture (consented) -> draft notes
   - Coordinator copilots for missing data and protocol deviations

6. **Analytics & Statistics**
   - Trial/site progress dashboards
   - Milestone tracking and operational KPIs
   - Statistical summaries by project and organization
   - Export for advanced biostatistics pipelines

7. **Billing & Subscriptions**
   - Seat + study + storage usage metering
   - Tiered subscription plans
   - Enterprise support and compliance add-ons

## Suggested Subscription Strategy

- **Starter:** small private practices and pilots
- **Growth:** multi-site institutions with patient media workflows
- **Enterprise:** custom compliance controls, dedicated environments, SLA

Pricing formula can be based on:
- Active studies
- Monthly patient submissions
- Media storage/egress
- AI inference usage

## Compliance and Security Direction

- HIPAA-aligned architecture and BAAs for all subprocessors
- Encryption in transit and at rest
- Full audit trails for PHI access and mutation
- SSO with Google Workspace (OIDC/SAML)
- Least-privilege RBAC and scoped API tokens

## Initial Roadmap

### Phase 1 (MVP)
- Tenant/study/site setup
- Patient invites + forms + eConsent
- Media upload (photo/video)
- Coordinator dashboards and exports

### Phase 2
- AI note drafting and protocol guidance
- Advanced analytics workspace
- Billing and subscription automation

### Phase 3
- Marketplace module ecosystem
- Partner integrations (EHR, CTMS, ePRO)
- Federated multi-organization analytics
