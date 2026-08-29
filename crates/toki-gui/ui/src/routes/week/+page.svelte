<main class="page">
  <header class="hero">
    <div class="eyebrow">Week</div>
    <div class="hero-value">Coming soon</div>
  </header>

  <section class="card">
    <p class="muted">
      Daily totals, top entities, and a week-over-week delta — not built yet. See
      CHECKLIST.md's Phase 6 section for what's left.
    </p>
  </section>
</main>

<style>
  .page {
    max-width: 640px;
    margin: 0 auto;
    padding: var(--gap-section) 24px;
    display: flex;
    flex-direction: column;
    gap: var(--gap-section);
  }

  .hero {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .eyebrow {
    font-size: 13px;
    color: var(--text-secondary);
  }

  .hero-value {
    font-size: var(--display-size);
    font-weight: 500;
    letter-spacing: -0.02em;
    color: var(--text-muted);
  }

  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-padding);
    background: var(--surface);
  }

  .muted {
    margin: 0;
    color: var(--text-secondary);
  }
</style>
