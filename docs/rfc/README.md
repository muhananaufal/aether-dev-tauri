# 📚 RFC Architecture Catalog & Master Index

Dokumentasi rancangan arsitektur, keputusan sistem (ADR), dan riwayat implementasi teknis untuk **Dev Control Center (Tauri)**.

| RFC ID | Inisiatif Fitur / Arsitektur | Domain / Modul | Target Branch | Status | Tanggal |
| :--- | :--- | :--- | :--- | :---: | :---: |
| `20260806` | [Rewrite Rust + Tauri v2 (Open Source, Lintas Platform)](20260806-dev-control-center-tauri.md) | Core / Seluruh Aplikasi | `feature/tauri-foundation` | `ACCEPTED` | 2026-08-06 |

### Panduan Siklus Status RFC

* `PROPOSED`: Sedang dalam tahap perancangan & review (menunggu "Gasskan").
* `ACCEPTED`: Telah disetujui user ("Gasskan"), siap/sedang dieksekusi.
* `IMPLEMENTED`: Seluruh batch task tuntas dan lolos Quality Gate.
* `SUPERSEDED`: Digantikan oleh dokumen RFC yang lebih baru (sertakan link ke RFC pengganti).

### Catatan Konteks

Proyek ini adalah penulisan ulang dari aplikasi PowerShell + WinForms di `C:\Resources\Tools\dev-control-center`.
Repositori lama **tidak disentuh** dan tetap berfungsi sebagai jalur mundur selama paritas fitur belum terverifikasi penuh.
