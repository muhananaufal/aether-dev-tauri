# RFC: Dev Control Center — Rewrite Rust + Tauri v2 (Open Source, Lintas Platform)

- **Status:** `ACCEPTED`
- **Tanggal:** 2026-08-06
- **Disetujui:** 2026-08-06 ("Gasskan")
- **Target Branch:** `feature/tauri-foundation`
- **Menggantikan:** `C:\Resources\Tools\dev-control-center` (PowerShell 5.1 + WinForms, 4220 baris) — repo lama TETAP hidup, tidak disentuh.

---

## 1. Konteks & Problem Statement (PRD Core)

### Latar Belakang & Urgensi

Aplikasi asal adalah GUI WinForms 4220 baris yang mengorkestrasi environment development lokal: workspace proyek, stack Docker 15 service, manajemen port, dan monitor log. Aplikasi ini bekerja dengan baik **untuk satu mesin** dan tidak bisa dipasang orang lain.

Audit forensik terhadap repo lama (2026-08-06) menemukan penghalang yang terukur:

| Kategori | Temuan terukur |
| :--- | :--- |
| Kredensial | `.env` berisi 18 kredensial, ter-commit sejak commit pertama `575113e` |
| Kredensial | 21 password literal di `control-center.ps1`, 2 di `backup-dbs.ps1` |
| Path | 9 path absolut `C:\Resources\Tools\dev-control-center` / `/mnt/c/...` |
| Path | 7 path `C:\Users\muhan\...` |
| Lingkungan | 50 pemanggilan `wsl -d Ubuntu -u root` |
| Higiene | Tidak ada `.gitignore`; tidak ada test suite |
| Cacat | 2 fungsi dipanggil tapi tidak pernah didefinisikan (`Set-ActivePhpJunction`, `Get-CustomDevIcon`) |
| Cacat | Injeksi shell/SQL: nama database dari textbox bebas diinterpolasi mentah ke `bash -c` |
| Cacat | `Stop-Process -Name "php","node","go"` membunuh seluruh proses node di sistem |

**Kesimpulan audit yang mengarahkan RFC ini:** yang menghalangi distribusi bukan bahasanya, melainkan ~50 asumsi lingkungan yang tertanam di seluruh kode. Rewrite tanpa membongkar asumsi itu hanya menghasilkan aplikasi yang tetap jalan di satu mesin, dengan waktu kompilasi.

### Scope Boundaries

**IN-SCOPE**

1. Paritas fitur 1:1 dengan aplikasi PowerShell, empat tab:
   - Workspace Projects (scan, deteksi stack, versi framework, status Git, Run Dev, Terminal, Config, editor, folder, browser)
   - Databases & Services (11 service, START/STOP, Import/Export DB untuk 5 engine)
   - Port & Process Manager (daftar port listening, kill per-PID, Kill Dev Ports, Free RAM, Local Domain Manager + generator Caddyfile)
   - Live Logs Monitor
2. Lapisan transport Docker ganda (WSL dan native) dengan deteksi otomatis + override manual.
3. Halaman Settings: setiap asumsi lingkungan jadi input, terisi otomatis oleh deteksi.
4. Registry service berbasis JSON sebagai sumber kebenaran tunggal.
5. Dukungan Windows, Linux, macOS.
6. Installer per platform + repo publik yang siap di-fork.

**EXPLICIT NON-GOALS**

1. **DILARANG menyentuh repo lama.** Tidak ada migrasi, tidak ada penulisan ulang riwayat Git di sana.
2. Tidak ada fitur baru di luar daftar paritas. Perbaikan cacat keamanan bukan fitur baru — itu paritas yang benar.
3. Tidak ada telemetri, tidak ada auto-update, tidak ada sinkronisasi cloud.
4. Tidak ada mode headless/CLI di v1 (lihat Opsi C §2 — sengaja ditolak untuk v1).
5. Tidak ada integrasi OS keyring untuk kredensial di v1. Kredensial tetap di `.env` yang di-generate acak dan di-gitignore.
6. Tidak ada dukungan Podman/containerd di v1 (arsitektur menyisakan ruang, implementasinya tidak).
7. Tidak ada penandatanganan kode berbayar (Apple Developer ID / EV cert). Rilis awal unsigned dengan instruksi bypass.

### Asumsi Epistemik & Skala (Hasil Gerbang Klarifikasi §2)

| Parameter | Nilai | Tingkat keyakinan |
| :--- | :--- | :--- |
| Jumlah pengguna serentak | 1 (aplikasi desktop single-user) | `fakta` |
| Jumlah proyek di-scan | 10–200 direktori, kedalaman 2 level | `inferensi` dari struktur `C:\Projects` repo lama |
| Jumlah service Docker dikelola | 11 default, dapat ditambah user via JSON | `fakta` |
| Frekuensi refresh status | 10 detik (dapat dikonfigurasi) | `fakta` dari perilaku repo lama |
| Target waktu render ulang daftar proyek | < 100 ms dari cache | `spekulasi` — belum ada baseline terukur |
| Target waktu scan dingin 200 proyek | < 3 detik | `spekulasi` — perlu benchmark di Batch 5 |
| Target ukuran installer | < 15 MB per platform | `inferensi` dari karakteristik Tauri v2 |
| Mode Docker | WSL **dan** native, auto-detect | keputusan user |
| Platform | Windows + Linux + macOS | keputusan user |
| Frontend | React + TypeScript + Tailwind | keputusan user |
| Registry service | JSON sejak awal | keputusan user |

**Asumsi yang WAJIB diverifikasi sebelum Batch 4:** apakah `docker compose` (plugin v2) tersedia di semua target, atau masih ada pengguna `docker-compose` (v1 standalone). Ini memengaruhi bentuk `CommandSpec`.

---

## 2. Eksplorasi Arsitektur & Trade-off Matrix (Anti-Yes-Man §4)

Tiga opsi disajikan netral. Keputusan ada pada user.

### Opsi A: Transport Abstraction + Thin Tauri Commands (Layered)

**Deskripsi Arsitektur.** Satu trait `CommandRunner` menjadi satu-satunya pintu keluar ke sistem. Dua implementasi (`NativeRunner`, `WslRunner`) dipilih saat startup oleh `probe`. Modul domain (`docker`, `project`, `ports`, `logs`) hanya bergantung pada trait, bukan pada `wsl.exe` atau `docker.exe`. Perintah Tauri adalah adaptor tipis: deserialisasi → panggil domain → serialisasi.

```text
[ React UI ]
     | invoke()  (IPC, JSON)
     v
[ commands/*.rs ]  ← adaptor tipis, nol business logic
     v
[ docker/ | project/ | ports/ | logs/ ]  ← domain, bergantung pada trait
     v
[ dyn CommandRunner ]  ← SATU pintu keluar
     |                         |
[ NativeRunner ]        [ WslRunner ]
  docker.exe              wsl -d <distro>
```

- **Kelebihan (≥3):**
  1. Seluruh domain dapat diuji tanpa Docker terpasang, memakai `MockCommandRunner` dari `mockall`. Ini satu-satunya opsi yang membuat CI di GitHub Actions bermakna.
  2. Penambahan transport ketiga (Podman, SSH remote) menyentuh satu berkas, bukan 50 titik panggilan.
  3. Perbaikan injeksi terjadi di satu batas (`CommandSpec` berbasis vektor argumen), bukan tersebar di setiap pemanggil.
  4. Batas error jelas: error transport dibungkus jadi error domain sebelum mencapai IPC, sehingga detail sistem tidak bocor ke UI.
- **Kekurangan & Failure Modes (≥3):**
  1. Struktur di muka lebih berat — ada trait, ada dua impl, ada probe, sebelum satu tombol pun berfungsi.
  2. `dyn CommandRunner` berarti dynamic dispatch dan `Arc` di mana-mana; menelusuri satu alur saat debug melewati lebih banyak lapisan.
  3. Abstraksi bisa bocor: `docker exec -i` dengan stdin streaming dan `docker logs -f` yang long-lived tidak muat dalam satu bentuk `run() -> Output`. Trait butuh tiga bentuk (`run`, `stream`, `pipe`), dan itu menambah permukaan.
  4. Risiko over-abstraction bila ternyata selamanya hanya ada dua transport.
- **Reversibility:** `Two-Way Door` — bila terbukti berlebihan, trait bisa di-inline kembali secara mekanis.

### Opsi B: Perintah Tauri Datar, Transport Dipilih Per Panggilan

**Deskripsi Arsitektur.** Tidak ada trait. Setiap perintah Tauri membangun invokasinya sendiri, dengan percabangan `if is_wsl { ... } else { ... }` di tempat. Satu berkas per tab.

- **Kelebihan (≥3):**
  1. Tercepat sampai layar pertama berfungsi; tidak ada fondasi yang harus dibangun dulu.
  2. Satu perintah terbaca utuh dari atas ke bawah tanpa melompat antar berkas.
  3. Nol indireksi saat debug — apa yang tertulis itu yang dieksekusi.
  4. Cocok bila ternyata proyek berhenti di v1 dan tidak pernah tumbuh.
- **Kekurangan & Failure Modes (≥3):**
  1. Mereproduksi persis penyakit repo lama: percabangan lingkungan tersebar di puluhan titik. Audit menemukan 50 titik seperti itu; opsi ini menjamin jumlahnya kembali tumbuh.
  2. Tidak dapat diuji tanpa Docker + WSL terpasang di runner CI. Praktis berarti nol test otomatis.
  3. Perbaikan injeksi harus diterapkan berulang di setiap titik; satu yang terlewat cukup untuk membatalkan seluruh usaha.
  4. Menambah transport ketiga berarti menyunting setiap perintah.
- **Reversibility:** `Two-Way Door` secara teori, tetapi biayanya setara menulis ulang lapisan backend.

### Opsi C: Sidecar CLI + Tauri sebagai Cangkang UI Murni

**Deskripsi Arsitektur.** Seluruh logika dibangun sebagai binary CLI Rust mandiri (`dcc`) yang mengeluarkan JSON. Aplikasi Tauri memaketkannya sebagai *sidecar* dan berkomunikasi lewat proses anak.

- **Kelebihan (≥3):**
  1. CLI dapat dipakai berdiri sendiri — otomasi, CI, pengguna yang tidak mau GUI.
  2. Pemisahan UI dan logika bersifat fisik, bukan konvensi; mustahil bocor.
  3. CLI dapat diuji sebagai proses nyata dengan snapshot output, tanpa harness Tauri.
  4. Mengganti frontend (misal ke TUI) tidak menyentuh logika sama sekali.
- **Kekurangan & Failure Modes (≥3):**
  1. Serialisasi ganda: domain → JSON → proses → JSON → IPC → UI. Untuk log streaming dan progress bar ini menambah latensi dan kerumitan nyata.
  2. Dua binary yang harus dibangun, ditandatangani, diversikan, dan dijaga tetap kompatibel di tiga platform. Sidecar Tauri butuh penamaan per-triple target.
  3. Streaming (log `-f`, progress import) lewat batas proses jauh lebih rumit daripada lewat `app.emit()` in-process.
  4. Ukuran bundle naik; permukaan distribusi bertambah.
- **Reversibility:** mendekati `One-Way Door` — bentuk distribusi, CI, dan kontrak publik terbentuk mengikuti keputusan ini.

---

## 3. Spesifikasi Teknis & Desain Sistem Terpilih

> Spesifikasi berikut ditulis untuk **Opsi A**. Bila user memilih B atau C, seksi ini dirombak sebelum eksekusi.

### 3.1 Data Model & Strategi Migrasi Konfigurasi

Aplikasi ini **tidak memiliki database**. "Data model"-nya adalah berkas konfigurasi. Karena itu strategi migrasi skema DB (`Expand and Contract`) tidak berlaku dan digantikan versioning konfigurasi.

**Berkas dan lokasinya** (via crate `directories`, mengikuti konvensi tiap OS):

| Berkas | Lokasi | Isi | Di-commit? |
| :--- | :--- | :--- | :--- |
| `settings.toml` | config dir aplikasi | Seluruh setelan §6 hasil Gerbang Klarifikasi | Tidak |
| `services.json` | config dir aplikasi (default dibundel) | Registry 11 service + tambahan user | Default ya, milik user tidak |
| `domains.json` | direktori stack | Peta container → domain `.localhost` | Tidak |
| `.env` | direktori stack | Kredensial, di-generate acak saat pertama jalan | **Tidak — wajib di `.gitignore`** |
| `Caddyfile` | direktori stack | Hasil generate dari `domains.json` | Tidak |
| `docker-compose.yml` | direktori stack (dibundel) | Definisi 15 service, path relatif | Ya |

**Struktur `settings.toml`** (strongly-typed, `serde::Deserialize`):

```toml
schema_version = 1

[docker]
transport = "auto"        # auto | native | wsl
wsl_distro = ""           # kosong = distro default
wsl_user = ""             # kosong = user default, BUKAN root
compose_dir = ""          # kosong = direktori bundel aplikasi

[workspace]
roots = []                # kosong = belum dikonfigurasi, UI memandu
scan_depth = 2
cache_ttl_secs = 30

[toolchain]
git_bash = ""
preferred_shell = "auto"  # auto | zsh | bash | powershell
terminal = "auto"
php_search_paths = []
node_nvm_path = ""
go_root = ""

[behavior]
refresh_interval_secs = 10
log_poll_lines = 200
kill_dev_process_names = []      # DEFAULT KOSONG — lihat §3.4 Elevation of Privilege
protected_process_names = ["system", "svchost", "explorer", "csrss", "lsass", "smss", "wininit", "launchd", "systemd", "kernel_task"]

[[editors]]
label = "VS Code"
program = "code"
args = ["{path}"]
```

**Versioning konfigurasi.** Field `schema_version` wajib. Saat load, bila versi lebih rendah dari versi aplikasi, jalankan rantai fungsi migrasi murni (`migrate_v1_to_v2`) dan tulis ulang berkas setelah membuat salinan `settings.toml.bak`. Bila lebih tinggi, **tolak dengan pesan eksplisit** — jangan menebak dan jangan menimpa; pengguna mungkin menurunkan versi aplikasi.

**Fail-fast.** `figment` melapis: `Serialized::defaults()` ← hasil deteksi ← `Toml::file(settings.toml)` ← `Env::prefixed("DCC_")`. Kegagalan ekstraksi WAJIB menampilkan path field dan tipe yang salah apa adanya (`figment::Error` sudah membawa informasi ini) ke layar Settings, bukan panic diam.

### 3.2 Kontrak IPC & Interface Internal

**Kontrak transport (dikunci di Batch 1):**

```rust
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,          // vektor, BUKAN string — lihat §3.4 Tampering
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub timeout: Option<Duration>,
}

#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Jalankan sampai selesai, kumpulkan output.
    async fn run(&self, spec: &CommandSpec) -> Result<CommandOutput, ExecError>;

    /// Jalankan proses long-lived, alirkan baris stdout/stderr ke channel.
    async fn stream(&self, spec: &CommandSpec, tx: mpsc::Sender<LogLine>) -> Result<StreamHandle, ExecError>;

    /// Jalankan dengan stdin yang di-feed pemanggil dan stdout yang dikonsumsi pemanggil.
    /// Dipakai import/export DB — menggantikan pipeline `bash -c` sepenuhnya.
    async fn pipe(&self, spec: &CommandSpec, io: PipeIo) -> Result<CommandOutput, ExecError>;

    fn transport(&self) -> Transport;
    fn capabilities(&self) -> Capabilities;   // mis. supports_page_cache_drop
}
```

`WslRunner` membungkus: `wsl.exe -d <distro> [-u <user>] -- <program> <args...>`. Bentuk `--` memakai vektor argumen dan **tidak melewati shell**, sehingga tidak ada permukaan injeksi. `bash -c` hanya boleh dipakai untuk operasi yang benar-benar butuh shell (`sysctl` + `drop_caches`), dengan argumen konstan tanpa interpolasi input pengguna.

**Kontrak IPC (Rust → React).** Semua DTO memakai `#[derive(Serialize, TS)]` dengan `ts-rs`, sehingga tipe TypeScript **di-generate dari Rust**, bukan ditulis dua kali. Ini menutup kelas bug drift kontrak.

**Error IPC:**

```rust
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "code", content = "detail")]
pub enum AppError {
    #[error("Docker tidak terdeteksi di lingkungan ini")]
    DockerUnavailable,
    #[error("Distro WSL '{0}' tidak ditemukan")]
    WslDistroMissing(String),
    #[error("Nama tidak valid: {0}")]
    InvalidIdentifier(String),
    #[error("Perintah gagal (exit {code})")]
    CommandFailed { code: i32, stderr_excerpt: String },
    #[error("Konfigurasi tidak valid pada '{field}': {reason}")]
    ConfigInvalid { field: String, reason: String },
    #[error("Operasi tidak didukung transport aktif")]
    UnsupportedOnTransport,
}
```

Frontend memakai `code` bermesin untuk percabangan, bukan regex atas string. `stderr_excerpt` dipotong 2 KB dan disaring melalui redaksi (§3.4).

**Backward compatibility.** Ini aplikasi single-binary tanpa klien eksternal; kompatibilitas yang dijaga hanyalah `schema_version` konfigurasi dan format `services.json`.

### 3.3 Konkurensi, Race Condition & Failure Domain

| Risiko | Penanganan |
| :--- | :--- |
| Perintah Tauri sinkron memblokir event loop WRY → UI freeze | SELURUH perintah `async fn`. Proses eksternal via `tokio::process::Command`. Kerja CPU/IO sinkron (scan disk 200 direktori) via `tokio::task::spawn_blocking`. |
| Refresh 10 detik menumpuk saat Docker lambat | Guard `AtomicBool` + `tokio::sync::Semaphore(1)` per jenis refresh. Tick baru saat masih berjalan → dilewati, bukan diantrekan. |
| Lock contention state global | State dipecah granular: `Arc<RwLock<ProjectCache>>`, `Arc<RwLock<ServiceRegistry>>`, `Arc<Mutex<StreamRegistry>>` terpisah. **DILARANG** satu `GlobalState` tunggal. |
| Lock ditahan melintasi `.await` | Wajib `tokio::sync::{Mutex, RwLock}`. `std::sync::Mutex` hanya untuk seksi kritis yang tidak mengandung `.await`. |
| Log stream membanjiri UI | `mpsc::channel(1024)` bounded (bukan unbounded). Producer melambat sendiri saat penuh. UI menyimpan maksimum `log_poll_lines` baris, sisanya dibuang dari kepala. |
| Stream yatim setelah ganti tab / tutup window | `StreamRegistry` memegang `StreamHandle` per sumber. Ganti sumber → `abort()` + `kill()` proses anak lama sebelum memulai yang baru. Handle juga di-drop pada `WindowEvent::Destroyed`. |
| Panic di `spawn_blocking` | `JoinError` ditangani eksplisit; `is_panic()` di-log lewat `tracing::error!` lalu dipetakan ke `AppError`, bukan `unwrap()`. |
| Import/export dibatalkan di tengah | Proses anak di-`kill()`. Untuk export, berkas parsial DIHAPUS (repo lama sudah benar di sini: menolak berkas < 25 byte). Untuk import, keadaan DB tidak dijamin — UI WAJIB memperingatkan sebelum mulai. |
| Dua window memicu operasi destruktif serentak | Operasi destruktif (import, stop service, kill proses) memegang `Semaphore(1)` global bernama; permintaan kedua ditolak dengan `AppError::Busy`. |

**Degradasi.** Deteksi transport gagal → aplikasi tetap terbuka, seluruh tab Docker menampilkan keadaan kosong dengan tautan langsung ke Settings. **DILARANG** menutup aplikasi atau menampilkan dialog modal yang memblokir.

### 3.4 STRIDE Threat Model & Security Perimeter

Model ancaman untuk aplikasi desktop lokal berbeda dari layanan web, tetapi tidak kosong. Aplikasi ini menjalankan perintah OS sewenang-wenang atas nama pengguna.

| Vektor STRIDE | Skenario nyata pada aplikasi ini | Mitigasi arsitektural |
| :--- | :--- | :--- |
| **Spoofing** | Konten remote di webview memanggil `invoke()` dan mengeksekusi perintah Docker. | Tauri v2 Capabilities dengan allowlist perintah eksplisit. CSP ketat, `default-src 'self'`. **DILARANG** memuat URL remote atau `<iframe>` pihak ketiga. Verifikasi `Window::url()` pada perintah destruktif. |
| **Tampering** | `settings.toml` / `services.json` disunting agar `program` menunjuk ke biner berbahaya, lalu aplikasi mengeksekusinya. | `program` dari konfigurasi WAJIB lolos validasi: harus berupa nama pada `PATH` atau path absolut yang ada dan dapat dieksekusi. Nilai dari registry service **tidak pernah** dipakai membangun string shell. Seluruh eksekusi lewat vektor argumen. |
| **Tampering** | Nama database dari input bebas diinterpolasi ke perintah (cacat nyata repo lama: `mysql -e "CREATE DATABASE IF NOT EXISTS $targetDb;"`). | Dua lapis: (a) identifier divalidasi ketat `^[A-Za-z_][A-Za-z0-9_-]{0,63}$` sebelum menyentuh apa pun; (b) tidak ada `bash -c` pada jalur dump — gzip dikerjakan in-process dengan `flate2`, data mengalir lewat stdin/stdout `docker exec -i`. Pipeline shell dihapus dari jalur ini sepenuhnya. |
| **Repudiation** | Pengguna kehilangan data setelah import menimpa database dan tidak ada jejak apa pun. | Audit log terstruktur (JSON, `tracing`) ke berkas di log dir untuk SETIAP operasi destruktif: import, stop, kill PID, tulis Caddyfile, generate `.env`. Memuat timestamp, operasi, target, exit code. |
| **Information Disclosure** | Kartu service repo lama menampilkan `Pass: secret_mysql_password` di layar — bocor ke setiap screenshot dan rekaman layar. | Kredensial di UI ditampilkan tersamar secara default, dibuka per-item lewat aksi eksplisit, dengan tombol salin yang tidak me-render nilainya. |
| **Information Disclosure** | Kredensial masuk ke log atau ke `stderr_excerpt` yang dikirim ke UI. | Layer redaksi `tracing` + penyaring pada `stderr_excerpt`: pola `-p<...>`, `--password=`, `PASSWORD=`, dan seluruh nilai yang dikenal dari `.env` diganti `***`. Struct config meng-override `Debug` secara manual. |
| **Denial of Service** | Payload IPC raksasa dari frontend memicu OOM di backend (peringatan eksplisit dokumen Tauri v2). | Batas panjang pada setiap field string DTO saat deserialisasi. Transfer berkas besar (dump DB) **tidak pernah** melewati IPC — hanya path yang dikirim, data mengalir di sisi Rust. |
| **Denial of Service** | `docker logs -f` pada container cerewet membanjiri channel dan UI. | Channel bounded 1024 + pemotongan buffer UI + throttle emisi event (batch per 100 ms, bukan per baris). |
| **Elevation of Privilege** | Repo lama memakai `wsl -u root` pada 50 pemanggilan. Sebagian besar tidak membutuhkannya. | Default `wsl_user` KOSONG (user default distro). Root hanya diminta untuk operasi yang benar-benar memerlukannya (drop page cache), dan tombolnya disembunyikan bila transport tidak mendukung. Aplikasi **DILARANG** meminta elevasi saat startup. |
| **Elevation of Privilege** | "Kill Dev Ports" membunuh seluruh proses `node` di sistem, termasuk language server dan agent lain. | `kill_dev_process_names` **default kosong**. UI menampilkan pratinjau daftar proses yang akan dimatikan beserta PID dan meminta konfirmasi. Allowlist proses terlindungi tetap berlaku dan tidak dapat dikosongkan. |

---

## 4. Rencana Eksekusi & Living Task Checklist (DAG & Batch Protocol §7)

> Kanban: maksimal 2 item `[/]` bersamaan. Setiap batch diakhiri commit atomik + Quality Gate sebelum batch berikutnya.

### Batch 0: Higiene Repositori (Langkah 0 `git-workflow` §3)
* **DependsOn:** `[]`
* **Goal:** `.gitignore` ada dan ter-commit SEBELUM berkas lain apa pun. Tidak ada kredensial yang pernah masuk staging.
- [ ] `[NEW]` `.gitignore` (mencakup `.env`, `.env.*`, `!.env.example`, `target/`, `node_modules/`, `dist/`, `*.log`)
- [ ] `git init` + `git checkout -b main` + commit `chore: add .gitignore before any other file`
- [ ] `[NEW]` `LICENSE` (MIT), `[NEW]` `README.md` kerangka
- [ ] Verifikasi: `git log --stat` menunjukkan `.gitignore` sebagai commit pertama

### Batch 1: Contract Locking & Scaffold
* **DependsOn:** `[Batch 0]`
* **Goal:** Kunci SELURUH interface sebelum satu baris business logic ditulis (§7 Contract-First).
- [ ] `[NEW]` scaffold Tauri v2 + React + TS + Tailwind + Vite
- [ ] `[NEW]` `src-tauri/src/error.rs` — `AppError`, `ExecError`, mapping ke IPC
- [ ] `[NEW]` `src-tauri/src/exec/mod.rs` — `CommandSpec`, `CommandRunner`, `Transport`, `Capabilities`
- [ ] `[NEW]` `src-tauri/src/model/` — SELURUH DTO + derive `ts-rs`
- [ ] `[NEW]` `resources/services.json` + skema + validator
- [ ] Verifikasi: `cargo check`, `cargo clippy -- -D warnings`, tipe TS ter-generate

### Batch 2: Exec Transport & Environment Probe
* **DependsOn:** `[Batch 1]`
* **Goal:** Kedua transport hidup dan terdeteksi otomatis; seluruh domain berikutnya dapat diuji tanpa Docker.
- [ ] `[NEW]` `src-tauri/src/exec/native.rs`
- [ ] `[NEW]` `src-tauri/src/exec/wsl.rs` (enumerasi distro, vektor argumen, tanpa shell)
- [ ] `[NEW]` `src-tauri/src/exec/probe.rs` (deteksi + urutan prioritas + override)
- [ ] `[NEW]` `src-tauri/src/exec/validate.rs` (validasi identifier, validasi `program`)
- [ ] Verifikasi: unit test + `MockCommandRunner`; `proptest` untuk validator identifier

### Batch 3: Config, Deteksi Toolchain & Settings
* **DependsOn:** `[Batch 2]`
* **Goal:** Setiap asumsi lingkungan jadi setelan yang terisi otomatis.
- [ ] `[NEW]` `src-tauri/src/config/mod.rs` (figment, fail-fast, `schema_version`)
- [ ] `[NEW]` `src-tauri/src/config/detect.rs` (PHP, Node/nvm, Go, Git Bash, terminal, editor)
- [ ] `[NEW]` `src-tauri/src/config/store.rs` (load/save, migrasi, backup)
- [ ] `[NEW]` generator `.env` acak saat pertama jalan
- [ ] Verifikasi: test layering + test migrasi + test penolakan `schema_version` lebih tinggi

### Batch 4: Domain Docker
* **DependsOn:** `[Batch 3]`
* **Goal:** Paritas tab Databases & Services, termasuk import/export tanpa pipeline shell.
- [ ] `[NEW]` `src-tauri/src/docker/compose.rs` (up/down/stop per service)
- [ ] `[NEW]` `src-tauri/src/docker/status.rs` (`ps`, `stats`, cek port TCP)
- [ ] `[NEW]` `src-tauri/src/docker/dump.rs` (5 engine, gzip in-process via `flate2`)
- [ ] `[NEW]` `src-tauri/src/docker/domain.rs` (generator Caddyfile + reload)
- [ ] Verifikasi: `insta` snapshot untuk keluaran Caddyfile; test dump dengan mock runner

### Batch 5: Domain Workspace Proyek
* **DependsOn:** `[Batch 3]`
* **Goal:** Paritas tab Workspace Projects.
- [ ] `[NEW]` `src-tauri/src/project/scan.rs` (`spawn_blocking`, cache TTL)
- [ ] `[NEW]` `src-tauri/src/project/stack.rs` (Laravel/CI3/CI4/Rust/Go/Python/Next/Vite/Node/Docker + versi framework)
- [ ] `[NEW]` `src-tauri/src/project/git.rs` (baca `.git/HEAD`, worktree, `status --porcelain`)
- [ ] `[NEW]` `src-tauri/src/project/launcher.rs` (isolasi PATH per proyek, lintas platform, fallback bash)
- [ ] Verifikasi: fixture direktori proyek sintetis untuk tiap stack; benchmark scan 200 direktori

### Batch 6: Ports & Live Logs
* **DependsOn:** `[Batch 2]`
* **Goal:** Paritas tab Ports dan Logs, dengan streaming sungguhan.
- [ ] `[NEW]` `src-tauri/src/ports/mod.rs` (`netstat2` + `sysinfo`, lintas platform, allowlist terlindungi)
- [ ] `[NEW]` `src-tauri/src/logs/stream.rs` (`docker logs -f` + tail berkas, mpsc bounded, `StreamRegistry`)
- [ ] `[NEW]` `src-tauri/src/audit.rs` (audit log operasi destruktif)
- [ ] Verifikasi: test siklus hidup stream (start → ganti → abort tanpa proses yatim)

### Batch 7: Frontend React
* **DependsOn:** `[Batch 4, Batch 5, Batch 6]`
* **Goal:** Empat tab + Settings, memakai tipe hasil generate.
- [ ] `[NEW]` shell aplikasi, tema gelap, navigasi tab, `lib/ipc.ts` terketik
- [ ] `[NEW]` `views/Projects.tsx`, `views/Services.tsx`
- [ ] `[NEW]` `views/Ports.tsx`, `views/Logs.tsx`
- [ ] `[NEW]` `views/Settings.tsx` (seluruh field §3.1 + tombol verifikasi per field kritis)
- [ ] Verifikasi: `tsc --noEmit`, `eslint`, `vitest` + Testing Library untuk komponen berlogika

### Batch 8: Packaging, CI & Dokumentasi Publik
* **DependsOn:** `[Batch 7]`
* **Goal:** Repo siap di-fork dan dipasang orang lain.
- [ ] `[NEW]` `.github/workflows/ci.yml` (fmt, clippy, nextest, tsc, eslint, vitest)
- [ ] `[NEW]` `.github/workflows/release.yml` (bundle 3 platform; macOS WAJIB di runner macOS — lihat catatan di bawah)
- [ ] `[NEW]` `justfile` (dev, build, lint, test, bundle)
- [ ] `[NEW]` `README.md` lengkap, `CONTRIBUTING.md`, `.env.example`, panduan prasyarat + wizard first-run
- [ ] Verifikasi: `cargo deny check`, `cargo audit`, `npm audit`; installer diuji pasang di Windows bersih

**Catatan silang-kompilasi.** Referensi `cross_compilation_zigbuild.md` menyatakan `cargo-zigbuild` optimal untuk aplikasi CLI tanpa UI, dan menautkan framework macOS dari host non-macOS bermasalah secara legal maupun teknis. Karena ini aplikasi GUI dengan WebView native, **matriks rilis WAJIB memakai runner per-OS asli**, bukan zigbuild dari satu host. Ini menaikkan waktu CI dan diterima sebagai konsekuensi keputusan tiga platform.

---

## 5. Observabilitas & Verifikasi Mutu (Quality Gate §6)

### Penyesuaian Day-0 Quintet

Protokol `rust-mastery/_protocol/greenfield.md` mensyaratkan Day-0 Quintet yang dirancang untuk **layanan backend**. Aplikasi ini desktop, tanpa server dan tanpa database sendiri. Penyimpangan dinyatakan eksplisit beserta alasannya, bukan dilewati diam-diam:

| Pilar Quintet | Status | Alasan |
| :--- | :--- | :--- |
| Container 3-tier (Postgres/Valkey, OTel, k6) | **Tidak berlaku sebagai infra aplikasi** | Aplikasi tidak punya DB atau endpoint. `docker-compose.yml` yang ada adalah *artefak yang dikelola aplikasi*, bukan infrastruktur aplikasi. Memasang OTel + k6 untuk GUI desktop single-user melanggar §6 KISS. |
| Config fail-fast | **Berlaku penuh** | figment + struct strongly-typed + kegagalan deskriptif sebelum window dibuka. |
| Graceful shutdown | **Berlaku penuh** | Pada `WindowEvent::Destroyed` dan exit: abort seluruh `StreamHandle`, kill proses anak, flush subscriber `tracing`. **DILARANG** `tokio::spawn` di dalam `Drop` (runtime sedang dibongkar). |
| Migrasi & seeder | **Diganti** | Tidak ada DB. Digantikan versioning + migrasi `settings.toml` (§3.1). |
| Task runner | **Berlaku penuh** | `justfile`. |

### Rencana Telemetri

Sistem ini bukan sistem terdistribusi; metrik RED dan distributed tracing tidak berlaku. Yang berlaku:

- **Structured logging** — `tracing` + `tracing-subscriber` layer JSON ke berkas berotasi di log dir OS. Field: `operation`, `transport`, `target`, `exit_code`, `duration_ms`.
- **Audit log** — subset khusus operasi destruktif (§3.4 Repudiation), berkas terpisah, tidak pernah dirotasi keluar tanpa sepengetahuan pengguna.
- **Redaksi** — layer penyaring wajib aktif sebelum sink mana pun.
- **Nol OpenTelemetry, nol Prometheus.** Menambahkannya untuk aplikasi desktop single-user adalah over-engineering.

### Rencana Pengujian

| Tingkat | Alat | Cakupan |
| :--- | :--- | :--- |
| Unit | `cargo test` | Deteksi stack, parsing versi, validasi identifier, migrasi config |
| Property | `proptest` | Validator identifier — invarian: input apa pun yang lolos tidak pernah mengandung metakarakter shell |
| Mock integration | `mockall` | Seluruh domain terhadap `MockCommandRunner`, tanpa Docker. Inilah yang membuat CI bermakna. |
| Snapshot | `insta` | Keluaran generator Caddyfile, serialisasi `services.json` |
| Frontend | `vitest` + Testing Library | Komponen berlogika (filter, pencarian, form Settings) |
| E2E manual | — | Checklist paritas 4 tab terhadap aplikasi PowerShell, dijalankan sebelum rilis |

**Non-Tautological Assertions.** DILARANG asersi berbentuk `assert!(result.is_ok())` tanpa memeriksa isi. Setiap test WAJIB memvalidasi nilai konkret atau varian error spesifik.

**Baseline regresi.** Repo lama **tidak punya test suite sama sekali** — fakta ini sudah diverifikasi. Karena itu tidak ada baseline hijau yang bisa dibandingkan. Baseline proyek ini dimulai dari nol di Batch 1 dan wajib naik monoton.

### Self-Review Checklist (§6) — diisi saat eksekusi, bukan sekarang

- N+1 query: `n/a — tidak ada database`
- OWASP / STRIDE: `<diisi per batch>`
- Race condition: `<diisi per batch>`
- Input validation: `<diisi per batch>`
- Memory leak: `<diisi per batch>`
- Task / process leak: `<diisi per batch>`

---

## 6. Prosedur Rollback & Strategi Rilis

Tidak ada production, tidak ada feature flag runtime, tidak ada canary ramp-up — ini aplikasi desktop yang dipasang pengguna. Padanan yang bermakna:

- **Jaring pengaman utama:** aplikasi PowerShell lama TETAP ADA dan berfungsi di `C:\Resources\Tools\dev-control-center`. Ia tidak disentuh sama sekali. Selama paritas belum terverifikasi penuh, itulah jalur mundur yang sesungguhnya.
- **Pemicu penghentian rilis:** satu saja dari empat tab gagal checklist paritas manual, ATAU `cargo clippy -- -D warnings` tidak bersih, ATAU ada kredensial terdeteksi di riwayat Git.
- **Rollback per batch:** setiap batch adalah commit atomik di micro-branch. Kegagalan Quality Gate → `git reset --soft HEAD~N` lalu commit ulang bersih (`git-workflow` §5). **DILARANG** commit `"fix: try again"` atau `"wip"`.
- **Rollback bagi pengguna:** rilis di-tag semver, installer versi lama tetap tersedia di GitHub Releases. Migrasi `settings.toml` selalu membuat `.bak`, sehingga menurunkan versi aplikasi tidak menghancurkan konfigurasi.
- **Kompatibilitas konfigurasi:** perubahan `schema_version` yang memaksa WAJIB disertai fungsi migrasi maju. Rilis yang menaikkan `schema_version` tanpa fungsi migrasi diblokir di CI.

---

## 7. Keputusan Terkunci (dijawab 2026-08-06)

| # | Pertanyaan | Keputusan | Alasan ringkas |
| :--- | :--- | :--- | :--- |
| 1 | Opsi arsitektur A, B, atau C? | **Opsi A** — Transport Abstraction + Thin Tauri Commands | Satu-satunya opsi yang membuat CI bermakna tanpa Docker/WSL di runner. Opsi B mereproduksi 50 titik percabangan yang membuat aplikasi lama tidak terdistribusi. Opsi C membeli CLI mandiri yang sudah masuk Non-Goals. |
| 2 | Compose v2 saja, atau v1 juga? | **v2 saja** | Compose v1 sudah EOL dan dicabut dari Docker Desktop. Mendukung keduanya berarti percabangan permanen di setiap panggilan. Deteksi sekali, gagal dengan pesan jelas bila absen. |
| 3 | Nama repo & namespace | **`muhananaufal/dev-control-center`** | Konsisten dengan nama lama, deskriptif. |
| 4 | Lisensi | **MIT** | Ini aplikasi, bukan library — alasan dual-license `MIT OR Apache-2.0` khas ekosistem Rust tidak berlaku. MIT memaksimalkan adopsi. Menaikkan ke Apache-2.0 masih mudah selama belum ada kontributor eksternal. |
