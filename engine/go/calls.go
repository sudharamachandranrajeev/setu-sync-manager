package main

import (
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/syncthing/syncthing/lib/config"
	"github.com/syncthing/syncthing/lib/events"
	"github.com/syncthing/syncthing/lib/protocol"
)

// args is what every call receives. Each call reads the fields it needs.
type args struct {
	Folder  string          `json:"folder"`
	Device  string          `json:"device"`
	ID      string          `json:"id"`
	Since   int             `json:"since"`
	Timeout int             `json:"timeout"`
	Kinds   string          `json:"kinds"`
	PerPage int             `json:"perpage"`
	Ignore  []string        `json:"ignore"`
	Value   json.RawMessage `json:"value"`
}

// call runs one request by name. Names match the methods of Setu's Rust `syncthing::Client`,
// and results have the JSON shape of the REST endpoint each method used before.
func call(method string, raw []byte) (any, error) {
	e, err := current()
	if err != nil {
		return nil, err
	}
	var a args
	if len(raw) > 0 {
		if err := json.Unmarshal(raw, &a); err != nil {
			return nil, fmt.Errorf("%s: bad arguments: %w", method, err)
		}
	}
	m, c := e.model, e.cfg
	switch method {
	case "my_id":
		return e.myID.String(), nil
	case "folders":
		return c.FolderList(), nil
	case "devices":
		return c.DeviceList(), nil
	case "default_folder":
		return c.DefaultFolder(), nil
	case "options":
		return c.Options(), nil

	case "put_device":
		d := c.DefaultDevice().Copy()
		if err := json.Unmarshal(a.Value, &d); err != nil {
			return nil, err
		}
		return nil, e.modify(func(cfg *config.Configuration) { cfg.SetDevice(d) })
	case "put_folder":
		f := c.DefaultFolder().Copy()
		if err := json.Unmarshal(a.Value, &f); err != nil {
			return nil, err
		}
		return nil, e.modify(func(cfg *config.Configuration) { cfg.SetFolder(f) })
	case "patch_folder":
		f, ok := c.Folder(a.ID)
		if !ok {
			return nil, fmt.Errorf("no folder %q", a.ID)
		}
		if err := json.Unmarshal(a.Value, &f); err != nil {
			return nil, err
		}
		return nil, e.modify(func(cfg *config.Configuration) { cfg.SetFolder(f) })
	case "delete_folder":
		w, err := c.RemoveFolder(a.ID)
		if err != nil {
			return nil, err
		}
		w.Wait()
		return nil, c.Save()
	case "patch_options":
		o := c.Options()
		if err := json.Unmarshal(a.Value, &o); err != nil {
			return nil, err
		}
		return nil, e.modify(func(cfg *config.Configuration) { cfg.Options = o })

	case "ignores":
		lines, patterns, err := m.LoadIgnores(a.Folder)
		return map[string]any{"ignore": lines, "expanded": patterns, "error": errText(err)}, nil
	case "set_ignores":
		return nil, m.SetIgnores(a.Folder, a.Ignore)
	case "scan":
		return nil, m.ScanFolderSubdirs(a.Folder, nil)

	case "connections":
		return m.ConnectionStats(), nil
	case "folder_status":
		return e.fss.Summary(a.Folder)
	case "completion":
		dev := protocol.LocalDeviceID
		if a.Device != "" {
			if dev, err = protocol.DeviceIDFromString(a.Device); err != nil {
				return nil, err
			}
		}
		comp, err := m.Completion(dev, a.Folder)
		if err != nil {
			return nil, err
		}
		return comp.Map(), nil
	case "versions":
		return m.GetFolderVersions(a.Folder)
	case "need":
		progress, queued, rest, err := m.NeedFolderFiles(a.Folder, 1, perPage(a.PerPage))
		if err != nil {
			return nil, err
		}
		return map[string]any{"progress": files(progress), "queued": files(queued), "rest": files(rest), "page": 1, "perpage": perPage(a.PerPage)}, nil
	case "local_changed":
		fs, err := m.LocalChangedFolderFiles(a.Folder, 1, perPage(a.PerPage))
		if err != nil {
			return nil, err
		}
		return map[string]any{"files": files(fs), "page": 1, "perpage": perPage(a.PerPage)}, nil
	case "folder_errors":
		errs, err := m.FolderErrors(a.Folder)
		if err != nil {
			return nil, err
		}
		if n := perPage(a.PerPage); len(errs) > n {
			errs = errs[:n]
		}
		return map[string]any{"folder": a.Folder, "errors": errs, "page": 1, "perpage": perPage(a.PerPage)}, nil

	case "events":
		mask := eventMask(a.Kinds)
		if mask == 0 {
			return nil, fmt.Errorf("events: no known event type in %q", a.Kinds)
		}
		timeout := time.Duration(max(a.Timeout, 0)) * time.Second
		return e.sub(mask).Since(a.Since, []events.Event{}, timeout), nil
	}
	return nil, fmt.Errorf("unknown engine call %q", method)
}

// modify changes the config, waits until the engine has taken it in, and saves it.
func (e *engine) modify(f func(*config.Configuration)) error {
	w, err := e.cfg.Modify(f)
	if err != nil {
		return err
	}
	w.Wait()
	return e.cfg.Save()
}

func eventMask(kinds string) events.EventType {
	var mask events.EventType
	for _, k := range strings.Split(kinds, ",") {
		mask |= events.UnmarshalEventType(strings.TrimSpace(k))
	}
	return mask
}

func perPage(n int) int {
	if n < 1 {
		return 1 << 16
	}
	return n
}

// file is a file entry with the fields Setu reads from Syncthing's file lists.
type file struct {
	Name     string    `json:"name"`
	Type     string    `json:"type"`
	Size     int64     `json:"size"`
	Deleted  bool      `json:"deleted"`
	Modified time.Time `json:"modified"`
}

func files(fs []protocol.FileInfo) []file {
	out := make([]file, len(fs))
	for i, f := range fs {
		out[i] = file{Name: f.FileName(), Type: f.FileType().String(), Size: f.Size, Deleted: f.IsDeleted(), Modified: f.ModTime()}
	}
	return out
}

func errText(err error) any {
	if err == nil {
		return nil
	}
	return err.Error()
}
