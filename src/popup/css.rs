pub const CSS: &str = r#"
.tc-shell {
    width: 384px;
    max-width: 100%;
    min-height: 480px;
    max-height: 600px;
    display: flex;
    flex-direction: column;
    background: #faf9f7;
    color: #1e1e1e;
    font-family: 'Inter', ui-sans-serif, system-ui, sans-serif;
    font-size: 13px;
    line-height: 1.45;
    border: 1px solid #e0ddd5;
    border-radius: 14px;
    overflow: hidden;
}
.tc-header {
    padding: 14px 14px 12px;
    border-bottom: 1px solid #e0ddd5;
    background: linear-gradient(180deg, #f5f3ef 0%, #faf9f7 100%);
}
.tc-brand-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
}
.tc-logo {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 18px;
    padding: 0 2px;
}
.tc-logo-bar {
    width: 4px;
    height: 18px;
    border-radius: 2px;
    display: block;
}
.tc-logo-purple { background: #9b6dd6; }
.tc-logo-cyan   { background: #46a7b8; height: 14px; }
.tc-logo-orange { background: #e08a3c; height: 9px; }
.tc-brand {
    font-weight: 700;
    font-size: 15px;
    letter-spacing: -0.2px;
}
.tc-sub {
    color: #6b6a68;
    font-size: 11.5px;
}
.tc-run {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 10px 12px;
    border-radius: 10px;
    border: none;
    cursor: pointer;
    background: #c2683a;
    color: #fff;
    font-weight: 600;
    font-size: 13.5px;
    letter-spacing: -0.1px;
    box-shadow: 0 1px 0 rgba(255,255,255,.14) inset, 0 4px 14px rgba(194,104,58,.35);
}
.tc-run:hover:not(:disabled) { filter: brightness(1.06); }
.tc-run:active:not(:disabled) { transform: translateY(1px); }
.tc-run:disabled {
    filter: saturate(.6) brightness(.95);
    cursor: default;
}
.tc-spin-icon {
    display: inline-block;
    animation: tc-spin 1s linear infinite;
}
@keyframes tc-spin { to { transform: rotate(360deg); } }
.tc-last-run {
    text-align: center;
    color: #6b6a68;
    font-size: 10.5px;
    margin-top: 8px;
    letter-spacing: .2px;
}
.tc-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.tc-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 20px;
    color: #6b6a68;
    text-align: center;
}
.tc-error { color: #e05a52; }
.tc-error-detail { font-size: 11px; opacity: .7; }
.tc-spinner {
    width: 24px;
    height: 24px;
    border: 3px solid #e0ddd5;
    border-top-color: #c2683a;
    border-radius: 50%;
    animation: tc-spin .8s linear infinite;
}
.tc-group {
    background: #ffffff;
    border: 1px solid #ece9e3;
    border-left: 3px solid #9b6dd6;
    border-radius: 12px;
    overflow: hidden;
    flex-shrink: 0;
}
.tc-group-head {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 9px 10px;
}
.tc-chev {
    background: none;
    border: none;
    color: #6b6a68;
    cursor: pointer;
    padding: 0;
    display: flex;
    font-size: 11px;
}
.tc-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex-shrink: 0;
    display: inline-block;
}
.tc-name {
    font-weight: 600;
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    color: #1e1e1e;
}
.tc-name-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    color: #1e1e1e;
    cursor: pointer;
    padding: 0;
    flex: 1;
    min-width: 0;
    text-align: left;
}
.tc-name-btn:hover .tc-pencil {
    opacity: 1;
}
.tc-pencil {
    color: #6b6a68;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity .15s;
    font-size: 12px;
    line-height: 1;
}
.tc-name-input {
    flex: 1;
    background: #ffffff;
    border: 1px solid #c2683a;
    border-radius: 6px;
    color: #1e1e1e;
    padding: 4px 7px;
    font-size: 13px;
    font-weight: 600;
    outline: none;
    min-width: 0;
}
.tc-count {
    margin-left: auto;
    background: #f0eeeb;
    color: #6b6a68;
    font-size: 11px;
    font-weight: 600;
    min-width: 20px;
    height: 20px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 6px;
    flex-shrink: 0;
}
.tc-group-body {
    padding: 4px 10px 11px;
    border-top: 1px solid #ece9e3;
}
.tc-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 10px;
}
.tc-row-label {
    color: #6b6a68;
    font-size: 11px;
    font-weight: 600;
    width: 56px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
}
.tc-palette {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
}
.tc-swatch {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    border: 2px solid transparent;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    color: #fff;
    font-size: 10px;
    font-weight: 700;
}
.tc-swatch:hover { transform: scale(1.08); }
.tc-theme-input {
    flex: 1;
    background: #ffffff;
    border: 1px solid #ece9e3;
    border-radius: 7px;
    color: #1e1e1e;
    padding: 7px 9px;
    font-size: 12.5px;
    outline: none;
}
.tc-theme-input::placeholder { color: #a09d98; }
.tc-theme-hint {
    color: #6b6a68;
    font-size: 10.5px;
    margin: 6px 0 0;
    padding-left: 66px;
    line-height: 1.4;
}
.tc-tab-list {
    list-style: none;
    margin: 11px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
}
.tc-tab {
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-rows: auto auto;
    column-gap: 8px;
    row-gap: 0;
    padding: 6px 8px;
    border-radius: 7px;
    background: #f5f3ef;
}
.tc-favi {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    grid-row: 1 / 3;
    align-self: center;
    flex-shrink: 0;
    display: inline-block;
}
.tc-favi-other { background: #d0cdc8; }
.tc-tab-title {
    font-size: 12.5px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.tc-tab-url {
    font-size: 10.5px;
    color: #6b6a68;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.tc-other {
    background: transparent;
    border: 1px dashed #d0cdc8;
    border-radius: 10px;
    padding: 9px 10px;
}
.tc-other-head {
    display: flex;
    align-items: center;
    gap: 7px;
}
.tc-other-icon { font-size: 14px; }
.tc-other-title {
    font-weight: 600;
    font-size: 12.5px;
    color: #6b6a68;
}
.tc-other-tab { opacity: .7; }
::-webkit-scrollbar { width: 9px; }
::-webkit-scrollbar-thumb { background: #d0cdc8; border-radius: 6px; border: 2px solid #faf9f7; }
::-webkit-scrollbar-track { background: transparent; }

/* ── New group creation ── */
.tc-new-group-area {
    margin-top: 8px;
}
.tc-new-group-btn {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 12px;
    border-radius: 8px;
    border: 1px dashed #d0cdc8;
    background: transparent;
    color: #6b6a68;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background .15s, color .15s;
}
.tc-new-group-btn:hover {
    background: #f0eeeb;
    color: #1e1e1e;
    border-color: #c2683a;
}
.tc-new-group-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.tc-new-group-input {
    background: #ffffff;
    border: 1px solid #c2683a;
    border-radius: 8px;
    color: #1e1e1e;
    padding: 8px 10px;
    font-size: 12.5px;
    outline: none;
}
.tc-new-group-input::placeholder { color: #a09d98; }
.tc-new-group-theme {
    background: #ffffff;
    border: 1px solid #ece9e3;
    border-radius: 8px;
    color: #1e1e1e;
    padding: 8px 10px;
    font-size: 12px;
    outline: none;
    resize: vertical;
    font-family: inherit;
    line-height: 1.4;
}
.tc-new-group-theme:focus { border-color: #c2683a; }
.tc-new-group-theme::placeholder { color: #a09d98; }
.tc-create-btn {
    padding: 8px 14px;
    border-radius: 8px;
    border: none;
    background: #c2683a;
    color: #fff;
    font-weight: 600;
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
}
.tc-create-btn:hover:not(:disabled) { filter: brightness(1.06); }
.tc-create-btn:active:not(:disabled) { transform: translateY(1px); }
.tc-create-btn:disabled {
    opacity: 0.5;
    cursor: default;
}

/* ── Guided empty state ── */
.tc-empty-guided {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 20px;
    color: #6b6a68;
    text-align: center;
}
.tc-empty-guided p {
    margin: 0;
    line-height: 1.5;
}
.tc-create-guide {
    margin-top: 8px;
    padding: 10px 20px;
    font-size: 13px;
}

/* ── Dissolve button ── */
.tc-dissolve-btn {
    display: block;
    width: 100%;
    margin-top: 10px;
    padding: 7px 12px;
    border-radius: 8px;
    border: 1px solid #e8d8d0;
    background: transparent;
    color: #b06050;
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    transition: background .15s, border-color .15s;
}
.tc-dissolve-btn:hover {
    background: rgba(224,90,82,.06);
    border-color: #c2683a;
    color: #c2683a;
}

/* ── Download progress / status ── */
.tc-download-status {
    text-align: center;
    color: #6b6a68;
    font-size: 10.5px;
    margin-top: 4px;
}

/* ── Onboarding screen ── */
.tc-onboarding {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 20px 16px 16px;
    flex: 1;
    overflow-y: auto;
}
.tc-onboarding-title {
    font-weight: 700;
    font-size: 16px;
    color: #1e1e1e;
    text-align: center;
    margin-bottom: 6px;
    letter-spacing: -0.2px;
    line-height: 1.3;
}
.tc-onboarding-sub {
    color: #6b6a68;
    font-size: 12px;
    text-align: center;
    margin-bottom: 16px;
    line-height: 1.4;
}
.tc-onboarding-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    width: 100%;
    margin-bottom: 16px;
}
.tc-onboarding-card {
    background: #ffffff;
    border: 1px solid #ece9e3;
    border-radius: 10px;
    padding: 10px 8px;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s, transform 0.1s;
    display: flex;
    flex-direction: column;
    gap: 4px;
    user-select: none;
    position: relative;
    min-height: 62px;
}
.tc-onboarding-card:hover {
    border-color: #c2683a;
    background: #faf9f7;
}
.tc-onboarding-card:active {
    transform: scale(0.97);
}
.tc-onboarding-card--selected {
    border-color: #c2683a;
    background: rgba(194,104,58,.08);
}
.tc-onboarding-card--selected:hover {
    background: rgba(194,104,58,.12);
}
.tc-onboarding-card-name {
    font-weight: 600;
    font-size: 11.5px;
    color: #1e1e1e;
    line-height: 1.2;
}
.tc-onboarding-card-preview {
    font-size: 10px;
    color: #6b6a68;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
}
.tc-onboarding-check {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #c2683a;
    color: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 10px;
    font-weight: 700;
    line-height: 1;
}
.tc-onboarding-cta {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 12px 16px;
    border-radius: 10px;
    border: none;
    cursor: pointer;
    background: #c2683a;
    color: #fff;
    font-weight: 600;
    font-size: 14px;
    letter-spacing: -0.1px;
    box-shadow: 0 1px 0 rgba(255,255,255,.14) inset, 0 4px 14px rgba(194,104,58,.35);
    margin-bottom: 10px;
}
.tc-onboarding-cta:hover:not(:disabled) { filter: brightness(1.06); }
.tc-onboarding-cta:active:not(:disabled) { transform: translateY(1px); }
.tc-onboarding-cta:disabled {
    filter: saturate(.6) brightness(.95);
    cursor: default;
}
.tc-onboarding-skip {
    background: none;
    border: none;
    color: #a09d98;
    font-size: 11.5px;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 6px;
    transition: color 0.15s;
}
.tc-onboarding-skip:hover {
    color: #6b6a68;
}

/* ── Edit toggle for group controls ── */
.tc-edit-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: 1px solid #ece9e3;
    border-radius: 6px;
    color: #6b6a68;
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    padding: 4px 10px;
    margin-top: 6px;
    transition: border-color 0.15s, color 0.15s;
}
.tc-edit-toggle:hover {
    border-color: #c2683a;
    color: #c2683a;
}
.tc-edit-panel {
    margin-top: 6px;
}
"#;
