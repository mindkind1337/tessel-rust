"""Validate the parity plan and render its owner-readable, offline dashboard."""
import argparse
import html
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def progress(plan):
    domains = plan["domains"]
    ids = {row["id"] for row in domains}
    assert domains and len(ids) == len(domains)
    for row in domains:
        assert not row["parity_verified"] or (
            row["implementation_complete"] and row.get("current_acceptance_artifacts")
        ), "Certification requires current acceptance evidence"
    if not plan["baseline_complete"]:
        return None
    scenarios = plan.get("scenarios", [])
    assert scenarios, "Missing scenario inventory"
    assert len({s["id"] for s in scenarios}) == len(scenarios), "Duplicate scenario"
    assert {s["domain"] for s in scenarios} == ids, "Inventory must cover every domain"
    for scenario in scenarios:
        assert scenario["domain"] in ids
        assert not scenario["parity_verified"] or (
            scenario["implementation_complete"] and scenario.get("current_acceptance_artifacts")
        ), "Verified scenarios require implementation and current evidence"
    return {
        "total": len(scenarios),
        "implemented": sum(s["implementation_complete"] for s in scenarios),
        "verified": sum(s["parity_verified"] for s in scenarios),
    }


def render(plan):
    domains = plan["domains"]
    metrics = progress(plan)
    percent = "Non mesuré" if metrics is None else f'{100 * metrics["verified"] / metrics["total"]:.1f} %'
    implementation = "Inventaire exhaustif des scénarios requis avant tout pourcentage." if metrics is None else (
        f'{metrics["implemented"]}/{metrics["total"]} scénarios implémentés '
        f'({100 * metrics["implemented"] / metrics["total"]:.1f} %); '
        f'{metrics["verified"]}/{metrics["total"]} vérifiés identiques. Chaque scénario compte une fois, sans pondération.'
    )
    verified = sum(row["parity_verified"] for row in domains)
    audited = sum(row["state"] != "audit_pending" for row in domains)
    esc = lambda value: html.escape(str(value))
    state = lambda value: {"audit_pending": "À auditer", "todo": "À faire", "doing": "En cours", "done": "Terminé", "blocked": "Bloqué"}.get(value, value)
    milestones = "".join(
        f'<tr><td>{esc(m["id"])}</td><td>{esc(m["target"] or "À estimer")}</td>'
        f'<td>{esc(m["name"])}</td><td>{esc(state(m["state"]))}</td><td>{esc(m["exit"])}</td></tr>'
        for m in plan["milestones"]
    )
    areas = "".join(
        f'<tr><td>{esc(d["id"])}</td><td>{esc(d["area"])}</td><td>{esc(state(d["state"]))}</td>'
        f'<td>{esc(d["acceptance"])}</td><td><details><summary>Évidence historique — à revalider</summary>'
        f'<p>{esc(d["historical_evidence"])}</p><p>{esc(d["evidence_source"])}</p></details>'
        f'<p>{esc(d["next_action"])}</p></td></tr>' for d in domains
    )
    reports = "".join(
        f'<article><h3>{esc(r["date"])} · {esc(r["build"])}</h3><p>{esc(r["completed"])}</p>'
        f'<p><b>Validation :</b> {esc(r["validation"])}</p><p><b>Reste :</b> {esc(r["remaining"])}</p></article>'
        for r in reversed(plan["reports"])
    )
    steps = "".join(f'<li>{esc(step)}</li>' for step in plan["workflow"])
    cadence = "".join(f'<li>{esc(step)}</li>' for step in plan["cadence"])
    return f'''<!doctype html>
<html lang="fr"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Tessel Rust — suivi de conversion</title>
<style>
body{{font:16px/1.55 system-ui;margin:0;background:#11151c;color:#e7edf5}}
main{{max-width:1400px;margin:auto;padding:32px}}h1,h2{{line-height:1.2}}a{{color:#83baff}}
.cards{{display:flex;flex-wrap:wrap;gap:16px}}.card,article{{background:#1c2430;padding:20px;border-radius:12px;flex:1;min-width:210px}}
.metric{{font-size:30px;font-weight:700}}.muted{{color:#b4c0d1}}.notice{{border-left:4px solid #e9bb58;padding:12px 20px;background:#29271f}}
table{{width:100%;border-collapse:collapse;margin:20px 0}}th,td{{padding:12px;text-align:left;vertical-align:top;border-bottom:1px solid #364252}}th{{background:#1c2430}}
.scroll{{overflow:auto}}summary{{cursor:pointer}}article{{margin:12px 0}}code{{overflow-wrap:anywhere}}progress{{width:100%;accent-color:#70b4ff}}
</style><main>
<h1>Tessel Rust — plan de conversion</h1>
<p class="muted">Début : {esc(plan['started'])} · Rapport : {esc(plan['updated'])} · Dernier lot vérifié : <code>{esc(plan['last_verified_rust_commit'])}</code></p>
<p>{esc(plan['objective'])}</p>
<div class="cards">
<div class="card"><div class="metric">{percent}</div>Scénarios vérifiés identiques<p class="muted">{implementation}</p></div>
<div class="card"><div class="metric">{audited}/{len(domains)}</div>Domaines réaudités dans ce plan<progress value="{audited}" max="{len(domains)}"></progress></div>
<div class="card"><div class="metric">{verified}/{len(domains)}</div>Domaines certifiés identiques<progress value="{verified}" max="{len(domains)}"></progress><p class="muted">Ce chiffre ne représente pas la quantité de code déjà écrite.</p></div>
</div>
<p class="notice">{esc(plan['metric_rule'])}</p><p>{esc(plan['scope_rule'])}</p>
<h2>Travail en cours</h2><p>{esc(plan['current_work']['status'])}</p>
<h2>Calendrier prévisionnel</h2><p>{esc(plan['schedule_note'])}</p>
<div class="scroll"><table><thead><tr><th>Jalon</th><th>Cible</th><th>Livrable</th><th>État</th><th>Critère de sortie</th></tr></thead><tbody>{milestones}</tbody></table></div>
<h2>Rapports de livraison</h2>{reports}
<h2>Processus et points de suivi</h2><ol>{steps}</ol><ul>{cadence}</ul>
<h2>Inventaire initial — {len(domains)} domaines</h2>
<p>Référence Vue : <code>{esc(plan['reference_commit'])}</code>. Les descriptions historiques servent à préparer l’audit, pas à déclarer la parité acquise.</p>
<div class="scroll"><table><thead><tr><th>ID</th><th>Domaine</th><th>État</th><th>Acceptation requise</th><th>Preuves et prochain travail</th></tr></thead><tbody>{areas}</tbody></table></div>
<p class="muted">Source : PARITY_PLAN.json · Génération : python scripts/parity_report.py · Contrôle : python scripts/parity_report.py --check</p>
</main></html>'''


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    content = render(json.loads((ROOT / "PARITY_PLAN.json").read_text(encoding="utf-8")))
    output = ROOT / "PARITY_REPORT.html"
    if args.check:
        assert output.read_text(encoding="utf-8") == content, "Report is stale; regenerate it"
    else:
        output.write_text(content, encoding="utf-8")
    print("Parity report validated" if args.check else "PARITY_REPORT.html generated")
