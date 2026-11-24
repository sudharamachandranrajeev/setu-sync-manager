// Setu's sync engine: Syncthing's own code, run inside the Setu process.
//
// Built with `go build -buildmode=c-archive` and linked into setu-app. There is no GUI, no REST
// API, no HTTP port and no API key: Setu calls the functions in exports.go directly. Arguments
// and results are JSON in the same shapes Syncthing's REST API uses, so Setu's Rust code reads
// them exactly as it read the REST answers before.
//
// This file and the others in engine/go are MPL-2.0 like the Syncthing code they wrap.
package main

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"reflect"
	"sync"
	"time"
	"unsafe"

	"github.com/thejerf/suture/v4"

	"github.com/syncthing/syncthing/lib/config"
	"github.com/syncthing/syncthing/lib/events"
	"github.com/syncthing/syncthing/lib/locations"
	"github.com/syncthing/syncthing/lib/model"
	"github.com/syncthing/syncthing/lib/protocol"
	"github.com/syncthing/syncthing/lib/svcutil"
	"github.com/syncthing/syncthing/lib/syncthing"
)

// How long deleted-file records are kept in the database (Syncthing's default).
const deleteRetention = 15 * 30 * 24 * time.Hour

// Largest log file kept from the last run; a bigger one is started afresh.
const maxLog = 8 << 20

// StartArgs is what Setu passes to start the engine.
type StartArgs struct {
	// Folder for keys, config and database. Created if missing.
	Home string `json:"home"`
	// Addresses the engine may listen on, e.g. "tcp://100.101.102.103:22000". Setu passes only
	// Tailscale addresses; an empty list is refused so the engine never listens everywhere.
	Listen []string `json:"listen"`
	// This computer's name for the other computers, used when the config is new.
	Name string `json:"name"`
}

type engine struct {
	app    *syncthing.App
	cfg    config.Wrapper
	ev     events.Logger
	model  model.Model
	fss    model.FolderSummaryService
	myID   protocol.DeviceID
	cancel context.CancelFunc
	log    *os.File

	subsMu sync.Mutex
	subs   map[events.EventType]events.BufferedSubscription
}

var (
	mu      sync.Mutex
	running *engine
	// Syncthing keeps paths and loggers in package globals, so one process runs one engine,
	// once. A second start after a stop is refused rather than half working.
	used bool
)

var errNotRunning = errors.New("the sync engine is not running")

func current() (*engine, error) {
	mu.Lock()
	defer mu.Unlock()
	if running == nil {
		return nil, errNotRunning
	}
	return running, nil
}

func start(a StartArgs) (map[string]any, error) {
	mu.Lock()
	defer mu.Unlock()
	if running != nil {
		return nil, errors.New("the sync engine is already running")
	}
	if used {
		return nil, errors.New("the sync engine was stopped; start Setu again to restart it")
	}
	if a.Home == "" {
		return nil, errors.New("no engine home given")
	}
	if len(a.Listen) == 0 {
		return nil, errors.New("no listen address given; the engine only listens on this computer's Tailscale address")
	}
	if err := os.MkdirAll(a.Home, 0o700); err != nil {
		return nil, err
	}
	used = true

	logFile := openLog(filepath.Join(a.Home, "engine.log"))
	slog.SetDefault(slog.New(slog.NewTextHandler(logFile, &slog.HandlerOptions{Level: slog.LevelInfo})))

	for _, d := range []locations.BaseDirEnum{locations.ConfigBaseDir, locations.DataBaseDir} {
		if err := locations.SetBaseDir(d, a.Home); err != nil {
			return nil, fmt.Errorf("engine home: %w", err)
		}
	}

	cert, err := syncthing.LoadOrGenerateCertificate(locations.Get(locations.CertFile), locations.Get(locations.KeyFile))
	if err != nil {
		return nil, fmt.Errorf("device key: %w", err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	early := suture.New("setu-early", svcutil.SpecWithDebugLogger())
	early.ServeBackground(ctx)
	ev := events.NewLogger()
	early.Add(ev)

	fail := func(err error) (map[string]any, error) {
		cancel()
		return nil, err
	}

	cfgPath := locations.Get(locations.ConfigFile)
	_, statErr := os.Stat(cfgPath)
	fresh := errors.Is(statErr, os.ErrNotExist)
	cfg, err := syncthing.LoadConfigAtStartup(cfgPath, cert, ev, false, true)
	if err != nil {
		return fail(fmt.Errorf("engine config: %w", err))
	}
	early.Add(cfg)
	myID := protocol.NewDeviceID(cert.Certificate[0])
	if err := enforce(cfg, myID, a, fresh); err != nil {
		return fail(err)
	}

	if err := syncthing.TryMigrateDatabase(ctx, deleteRetention); err != nil {
		return fail(fmt.Errorf("engine database upgrade: %w", err))
	}
	sdb, err := syncthing.OpenDatabase(locations.Get(locations.Database), deleteRetention)
	if err != nil {
		return fail(fmt.Errorf("engine database: %w", err))
	}

	app, err := syncthing.New(cfg, sdb, ev, cert, syncthing.Options{NoUpgrade: true})
	if err != nil {
		return fail(err)
	}
	if err := app.Start(); err != nil {
		return fail(fmt.Errorf("engine start: %w", err))
	}
	m, err := modelOf(app)
	if err != nil {
		app.Stop(svcutil.ExitError)
		return fail(err)
	}

	running = &engine{
		app:    app,
		cfg:    cfg,
		ev:     ev,
		model:  m,
		fss:    model.NewFolderSummaryService(cfg, m, myID, ev),
		myID:   myID,
		cancel: cancel,
		log:    logFile,
		subs:   make(map[events.EventType]events.BufferedSubscription),
	}
	// Subscribe now to what Setu reads, so events from the first seconds are not lost.
	for _, mask := range []events.EventType{
		events.ItemFinished | events.LocalIndexUpdated | events.FolderErrors,
		events.DownloadProgress,
	} {
		running.sub(mask)
	}
	return map[string]any{"myID": myID.String()}, nil
}

// enforce writes the settings Setu always runs with: no GUI or REST API, no discovery
// servers, relays, NAT traversal, usage reports, crash reports or upgrades, and listening
// only on the given addresses. It runs before the engine starts, so it never listens anywhere
// else, even for a moment. A fresh config also takes Setu's name for this computer.
func enforce(cfg config.Wrapper, myID protocol.DeviceID, a StartArgs, fresh bool) error {
	w, err := cfg.Modify(func(c *config.Configuration) {
		c.GUI.Enabled = false
		o := &c.Options
		o.RawListenAddresses = append([]string(nil), a.Listen...)
		o.GlobalAnnEnabled = false
		o.LocalAnnEnabled = false
		o.RelaysEnabled = false
		o.NATEnabled = false
		o.URAccepted = -1
		o.AutoUpgradeIntervalH = 0
		o.CREnabled = false
		o.StartBrowser = false
		if fresh && a.Name != "" {
			for i := range c.Devices {
				if c.Devices[i].DeviceID == myID {
					c.Devices[i].Name = a.Name
				}
			}
		}
	})
	if err != nil {
		return fmt.Errorf("engine settings: %w", err)
	}
	w.Wait()
	return cfg.Save()
}

// modelOf reaches the full model behind app.Internals, which Syncthing keeps unexported. It is
// the same object Syncthing's REST API uses. Syncthing is pinned to one version in go.mod and
// TestModelReachable fails if the field moves.
func modelOf(app *syncthing.App) (model.Model, error) {
	if app.Internals == nil {
		return nil, errors.New("engine internals missing")
	}
	f := reflect.ValueOf(app.Internals).Elem().FieldByName("model")
	if !f.IsValid() {
		return nil, errors.New("engine model not found (Syncthing version changed?)")
	}
	m, ok := reflect.NewAt(f.Type(), unsafe.Pointer(f.UnsafeAddr())).Elem().Interface().(model.Model)
	if !ok || m == nil {
		return nil, errors.New("engine model has an unexpected type")
	}
	return m, nil
}

func stop() error {
	mu.Lock()
	e := running
	running = nil
	mu.Unlock()
	if e == nil {
		return nil
	}
	e.app.Stop(svcutil.ExitSuccess)
	e.cancel()
	err := e.app.Error()
	slog.SetDefault(slog.New(slog.DiscardHandler))
	_ = e.log.Close()
	return err
}

// sub returns the buffered subscription for a set of event types, made the first time it is
// asked for (as the REST API does).
func (e *engine) sub(mask events.EventType) events.BufferedSubscription {
	e.subsMu.Lock()
	defer e.subsMu.Unlock()
	s, ok := e.subs[mask]
	if !ok {
		s = events.NewBufferedSubscription(e.ev.Subscribe(mask), 1000)
		e.subs[mask] = s
	}
	return s
}

func openLog(path string) *os.File {
	flags := os.O_CREATE | os.O_WRONLY | os.O_APPEND
	if st, err := os.Stat(path); err == nil && st.Size() > maxLog {
		_ = os.Rename(path, path+".old")
	}
	f, err := os.OpenFile(path, flags, 0o600)
	if err != nil {
		f, _ = os.OpenFile(os.DevNull, os.O_WRONLY, 0)
	}
	return f
}
