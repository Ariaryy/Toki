<main class="page">
  <header class="hero">
    <div class="eyebrow">Settings</div>
    <div class="hero-value">Coming soon</div>
  </header>

  <section class="card">
    <p class="muted">
      Alias editor, ignore list, idle threshold, and pause control — not built yet. The
      idle threshold also needs a place to live on the daemon side first; see
      CHECKLIST.md's Phase 6 section.
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
