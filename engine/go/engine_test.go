package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// One engine per process (see `used`), so every call is checked in this one test.
func TestEngine(t *testing.T) {
	home := t.TempDir()
	data := t.TempDir()
	if err := os.WriteFile(filepath.Join(data, "hello.txt"), []byte("hello"), 0o600); err != nil {
		t.Fatal(err)
	}

	if _, err := start(StartArgs{Home: home, Name: "test"}); err == nil {
		t.Fatal("started with no listen address")
	}
	res, err := start(StartArgs{Home: home, Listen: []string{"tcp://127.0.0.1:0"}, Name: "test"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := stop(); err != nil {
			t.Error(err)
		}
	})
	myID, _ := res["myID"].(string)
	if len(myID) < 50 {
		t.Fatalf("my id %q", myID)
	}

	// run calls the engine the way Setu does and decodes the JSON it would get.
	run := func(t *testing.T, method string, a map[string]any) any {
		t.Helper()
		raw, _ := json.Marshal(a)
		v, err := call(method, raw)
		if err != nil {
			t.Fatalf("%s: %v", method, err)
		}
		bs, err := json.Marshal(v)
		if err != nil {
			t.Fatalf("%s: %v", method, err)
		}
		var out any
		_ = json.Unmarshal(bs, &out)
		return out
	}

	t.Run("runs with Setu's settings", func(t *testing.T) {
		e, _ := current()
		if e.cfg.GUI().Enabled {
			t.Error("GUI / REST API enabled")
		}
		o := run(t, "options", nil).(map[string]any)
		for _, k := range []string{"globalAnnounceEnabled", "localAnnounceEnabled", "relaysEnabled", "natEnabled", "crashReportingEnabled", "startBrowser"} {
			if o[k] != false {
				t.Errorf("%s = %v", k, o[k])
			}
		}
		if o["urAccepted"] != float64(-1) || o["autoUpgradeIntervalH"] != float64(0) {
			t.Errorf("usage reports %v, upgrades %v", o["urAccepted"], o["autoUpgradeIntervalH"])
		}
		if got := o["listenAddresses"].([]any); len(got) != 1 || got[0] != "tcp://127.0.0.1:0" {
			t.Errorf("listen %v", got)
		}
		if _, err := os.Stat(filepath.Join(home, "cert.pem")); err != nil {
			t.Error("no device key in the engine home")
		}
	})

	t.Run("my id and devices", func(t *testing.T) {
		if run(t, "my_id", nil) != myID {
			t.Error("my_id differs from start")
		}
		devs := run(t, "devices", nil).([]any)
		if len(devs) != 1 || devs[0].(map[string]any)["deviceID"] != myID || devs[0].(map[string]any)["name"] != "test" {
			t.Errorf("devices %v", devs)
		}
		peer := "MFZWI3D-BONSGYC-YLTMRWG-C43ENR5-QXGZDMM-FZWI3DP-BONSGYY-LTMRWAD"
		run(t, "put_device", map[string]any{"value": map[string]any{"deviceID": peer, "name": "peer", "addresses": []string{"tcp://127.0.0.1:1"}}})
		if n := len(run(t, "devices", nil).([]any)); n != 2 {
			t.Errorf("%d devices after adding one", n)
		}
	})

	t.Run("folders", func(t *testing.T) {
		def := run(t, "default_folder", nil).(map[string]any)
		if def["type"] == nil {
			t.Fatalf("default folder %v", def)
		}
		run(t, "put_folder", map[string]any{"value": map[string]any{"id": "setu-f-test", "label": "Setu · test", "path": data, "type": "sendreceive", "versioning": map[string]any{"type": "trashcan", "params": map[string]any{"cleanoutDays": "30"}}}})
		run(t, "patch_folder", map[string]any{"id": "setu-f-test", "value": map[string]any{"label": "Setu · renamed"}})
		var found bool
		for _, f := range run(t, "folders", nil).([]any) {
			f := f.(map[string]any)
			if f["id"] == "setu-f-test" {
				found = true
				if f["label"] != "Setu · renamed" || f["path"] != data {
					t.Errorf("folder %v", f)
				}
			}
		}
		if !found {
			t.Fatal("folder not added")
		}
		run(t, "set_ignores", map[string]any{"folder": "setu-f-test", "ignore": []string{"*.iso"}})
		ig := run(t, "ignores", map[string]any{"folder": "setu-f-test"}).(map[string]any)
		if l := ig["ignore"].([]any); len(l) != 1 || l[0] != "*.iso" {
			t.Errorf("ignores %v", ig)
		}
		run(t, "scan", map[string]any{"folder": "setu-f-test"})
		deadline := time.Now().Add(20 * time.Second)
		for {
			st := run(t, "folder_status", map[string]any{"folder": "setu-f-test"}).(map[string]any)
			if st["state"] == "idle" && st["localFiles"] == float64(1) {
				if st["globalBytes"] != float64(5) {
					t.Errorf("status %v", st)
				}
				break
			}
			if time.Now().After(deadline) {
				t.Fatalf("folder never scanned: %v", st)
			}
			time.Sleep(200 * time.Millisecond)
		}
		comp := run(t, "completion", map[string]any{"folder": "setu-f-test"}).(map[string]any)
		if comp["completion"] != float64(100) {
			t.Errorf("completion %v", comp)
		}
		need := run(t, "need", map[string]any{"folder": "setu-f-test", "perpage": 200}).(map[string]any)
		if need["rest"] == nil || need["progress"] == nil || need["queued"] == nil {
			t.Errorf("need %v", need)
		}
		if lc := run(t, "local_changed", map[string]any{"folder": "setu-f-test"}).(map[string]any); lc["files"] == nil {
			t.Errorf("local changed %v", lc)
		}
		if fe := run(t, "folder_errors", map[string]any{"folder": "setu-f-test"}).(map[string]any); fe["folder"] != "setu-f-test" {
			t.Errorf("errors %v", fe)
		}
		if _, ok := run(t, "versions", map[string]any{"folder": "setu-f-test"}).(map[string]any); !ok {
			t.Error("versions is not a map")
		}
	})

	t.Run("connections and options", func(t *testing.T) {
		c := run(t, "connections", nil).(map[string]any)
		if c["total"] == nil || c["connections"] == nil {
			t.Errorf("connections %v", c)
		}
		run(t, "patch_options", map[string]any{"value": map[string]any{"maxRecvKbps": 500}})
		if o := run(t, "options", nil).(map[string]any); o["maxRecvKbps"] != float64(500) || o["relaysEnabled"] != false {
			t.Errorf("options after patch %v", o)
		}
	})

	t.Run("events", func(t *testing.T) {
		evs := run(t, "events", map[string]any{"since": 0, "timeout": 0, "kinds": "LocalIndexUpdated,ItemFinished,FolderErrors"}).([]any)
		if len(evs) == 0 {
			t.Fatal("no LocalIndexUpdated after the scan")
		}
		last := evs[len(evs)-1].(map[string]any)
		if last["type"] != "LocalIndexUpdated" || last["id"] == nil {
			t.Errorf("event %v", last)
		}
		since := int(last["id"].(float64))
		began := time.Now()
		if more := run(t, "events", map[string]any{"since": since, "timeout": 1, "kinds": "LocalIndexUpdated"}).([]any); len(more) != 0 {
			t.Errorf("unexpected events %v", more)
		}
		if time.Since(began) < 900*time.Millisecond {
			t.Error("events did not wait for the timeout")
		}
		if _, err := call("events", []byte(`{"kinds":"NoSuchEvent"}`)); err == nil {
			t.Error("unknown event type accepted")
		}
	})

	t.Run("delete folder and unknown calls", func(t *testing.T) {
		run(t, "delete_folder", map[string]any{"id": "setu-f-test"})
		for _, f := range run(t, "folders", nil).([]any) {
			if f.(map[string]any)["id"] == "setu-f-test" {
				t.Error("folder still there")
			}
		}
		if _, err := call("rest", nil); err == nil {
			t.Error("unknown call accepted")
		}
		if _, err := os.Stat(filepath.Join(data, "hello.txt")); err != nil {
			t.Error("removing the folder touched its files")
		}
	})
}
