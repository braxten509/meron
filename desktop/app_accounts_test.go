// Exercises the account settings contract exposed to the frontend.
// Password reconnect fixtures keep the existing identity and certificate choice.
// Preference cases check the optional sent-copy behavior and signature override.

package main

import "testing"

// The UI reads each account's signature override back from account.list, so a
// field missing from the Account struct reads as "setting lost on restart".
func TestAccountListKeepsSignatureOverride(t *testing.T) {
	app, _ := newMailHandlerTestApp(t, sidecarResponsePlan{Result: map[string]any{
		"accounts": []any{map[string]any{
			"id":        "acc",
			"email":     "user@example.com",
			"signature": map[string]any{"mode": "custom", "html": "<p>Ping</p>"},
		}},
	}})

	res, operationError := app.accountList()
	if operationError != nil {
		t.Fatal(operationError)
	}
	accounts := res.(map[string]any)["accounts"].([]Account)
	if len(accounts) != 1 {
		t.Fatalf("accounts = %#v, want one", accounts)
	}
	signature, _ := accounts[0].Signature.(map[string]any)
	if signature["mode"] != "custom" || signature["html"] != "<p>Ping</p>" {
		t.Fatalf("signature = %#v, want the stored override", accounts[0].Signature)
	}
}

func TestAccountSetSaveSentCopyMapsNullableValue(t *testing.T) {
	app, writer := newMailHandlerTestApp(t,
		sidecarResponsePlan{Result: map[string]any{"ok": true}},
		sidecarResponsePlan{Result: map[string]any{"ok": true}},
	)

	if _, operationError := app.accountSetSaveSentCopy(map[string]any{"id": "acc", "value": true}); operationError != nil {
		t.Fatal(operationError)
	}
	if _, operationError := app.accountSetSaveSentCopy(map[string]any{"id": "acc", "value": nil}); operationError != nil {
		t.Fatal(operationError)
	}

	if len(writer.calls) != 2 {
		t.Fatalf("sidecar calls = %#v, want two calls", writer.calls)
	}
	assertCall(t, writer.calls[0], "account.setSaveSentCopy", map[string]any{"account": "acc", "value": true})
	assertCall(t, writer.calls[1], "account.setSaveSentCopy", map[string]any{"account": "acc", "value": nil})
}

func TestAccountAddPasswordReconnectKeepsExistingID(t *testing.T) {
	app, writer := newMailHandlerTestApp(t,
		sidecarResponsePlan{Result: map[string]any{"accounts": []any{map[string]any{
			"id": "legacy-user1-mail-localhost", "email": "user1@mail.localhost",
		}}}},
		sidecarResponsePlan{Result: map[string]any{"ok": true}},
		sidecarResponsePlan{Result: map[string]any{"ok": true}},
	)
	result, operationError := app.accountAddPassword(map[string]any{
		"id": "legacy-user1-mail-localhost", "email": "user1@mail.localhost",
		"imap_host": "127.0.0.1", "smtp_host": "127.0.0.1",
		"username": "user1@mail.localhost", "password": "secret",
	})
	if operationError != nil {
		t.Fatal(operationError)
	}
	const want = "legacy-user1-mail-localhost"
	if got := writer.calls[1].Params["id"]; got != want {
		t.Fatalf("account.connect id = %#v, want %q", got, want)
	}
	if got := writer.calls[2].Params["account"]; got != want {
		t.Fatalf("watch.start account = %#v, want %q", got, want)
	}
	if got := result.(map[string]any)["account"].(Account).ID; got != want {
		t.Fatalf("returned account ID = %q, want %q", got, want)
	}
}

// An account whose server certificate the user accepted only stays reachable if
// the pin reaches the core with the connection settings.
func TestAccountAddPasswordForwardsAcceptedCertificate(t *testing.T) {
	app, writer := newMailHandlerTestApp(t,
		sidecarResponsePlan{Result: map[string]any{"accounts": []any{}}},
		sidecarResponsePlan{Result: map[string]any{"ok": true}}, // account.connect
		sidecarResponsePlan{Result: map[string]any{"ok": true}}, // watch.start
	)

	if _, operationError := app.accountAddPassword(map[string]any{
		"email":     "user@example.com",
		"imap_host": "127.0.0.1",
		"imap_port": 1143,
		"smtp_host": "127.0.0.1",
		"smtp_port": 1025,
		"username":  "user@example.com",
		"password":  "bridge-password",
		"cert_pin":  "  AB12  ",
	}); operationError != nil {
		t.Fatal(operationError)
	}

	if writer.calls[1].Method != "account.connect" {
		t.Fatalf("second call = %q, want account.connect", writer.calls[1].Method)
	}
	if got := writer.calls[1].Params["cert_pin"]; got != "AB12" {
		t.Fatalf("cert_pin = %#v, want the trimmed pin", got)
	}
}

// Accounts on servers with a normal certificate must not carry a pin at all —
// an empty one would be persisted and read back as "pinned to nothing".
func TestAccountAddPasswordOmitsEmptyCertificatePin(t *testing.T) {
	app, writer := newMailHandlerTestApp(t,
		sidecarResponsePlan{Result: map[string]any{"accounts": []any{}}},
		sidecarResponsePlan{Result: map[string]any{"ok": true}},
		sidecarResponsePlan{Result: map[string]any{"ok": true}},
	)

	if _, operationError := app.accountAddPassword(map[string]any{
		"email":     "user@example.com",
		"imap_host": "imap.example.com",
		"smtp_host": "smtp.example.com",
		"username":  "user@example.com",
		"password":  "secret",
		"cert_pin":  "",
	}); operationError != nil {
		t.Fatal(operationError)
	}

	if _, ok := writer.calls[1].Params["cert_pin"]; ok {
		t.Fatalf("params = %#v, want no cert_pin key", writer.calls[1].Params)
	}
}

// Accepting a certificate for an existing account must pin only the server the
// user was shown; the other server keeps whatever it already trusted.
func TestAccountSetCertPinForwardsOnlyTheAcceptedServer(t *testing.T) {
	app, writer := newMailHandlerTestApp(t, sidecarResponsePlan{Result: map[string]any{"ok": true}})

	if _, operationError := app.accountSetCertPin(map[string]any{
		"id":            "acc",
		"smtp_cert_pin": " 6f69d6a7 ",
	}); operationError != nil {
		t.Fatal(operationError)
	}

	if got := writer.calls[0].Params["smtp_cert_pin"]; got != "6f69d6a7" {
		t.Fatalf("smtp_cert_pin = %#v, want the trimmed pin", got)
	}
	if _, ok := writer.calls[0].Params["cert_pin"]; ok {
		t.Fatalf("params = %#v, want no cert_pin key", writer.calls[0].Params)
	}
}

func TestAccountSetCertPinRequiresAnAccount(t *testing.T) {
	app, _ := newMailHandlerTestApp(t)

	if _, operationError := app.accountSetCertPin(map[string]any{"cert_pin": "abc"}); operationError == nil {
		t.Fatal("want an error for a pin with no account")
	}
}

// The probe must take the same route as the connection that failed: an account
// on its own proxy is otherwise reached over a different path, or not at all.
func TestAccountProbeCertForwardsTheAccountProxy(t *testing.T) {
	app, writer := newMailHandlerTestApp(t, sidecarResponsePlan{Result: map[string]any{"certificate": map[string]any{}}})

	if _, operationError := app.accountProbeCert(map[string]any{
		"host":     "127.0.0.1",
		"port":     1025,
		"protocol": "smtp",
		"starttls": true,
		"proxy":    map[string]any{"mode": "socks5", "host": "127.0.0.1", "port": 9050},
	}); operationError != nil {
		t.Fatal(operationError)
	}

	params := writer.calls[0].Params
	proxy, _ := params["proxy"].(map[string]any)
	if proxy["mode"] != "socks5" || proxy["port"] != float64(9050) {
		t.Fatalf("proxy = %#v, want the account's own proxy", params["proxy"])
	}
	if params["protocol"] != "smtp" || params["starttls"] != true {
		t.Fatalf("params = %#v, want the submission server with STARTTLS", params)
	}
}

func TestAccountAddPasswordOmittedPassword(t *testing.T) {
	for _, tc := range []struct {
		name      string
		id        string
		email     string
		blank     bool
		wantError bool
	}{
		{"existing account keeps secret", "legacy", "user@example.com", false, false},
		{"new account requires secret", "", "user@example.com", false, true},
		{"changed email requires secret", "legacy", "other@example.com", false, true},
		{"explicit blank requires secret", "legacy", "user@example.com", true, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			app, writer := newMailHandlerTestApp(t,
				sidecarResponsePlan{Result: map[string]any{"accounts": []any{map[string]any{"id": "legacy", "email": "user@example.com"}}}},
				sidecarResponsePlan{Result: map[string]any{"ok": true}},
				sidecarResponsePlan{Result: map[string]any{"ok": true}},
			)
			payload := map[string]any{"id": tc.id, "email": tc.email, "imap_host": "imap.example.com", "smtp_host": "smtp.example.com", "username": tc.email}
			if tc.blank {
				payload["password"] = ""
			}
			_, operationError := app.accountAddPassword(payload)
			if (operationError != nil) != tc.wantError {
				t.Fatalf("error = %v, wantError %v", operationError, tc.wantError)
			}
			for _, call := range writer.calls {
				if call.Method == "account.connect" {
					if tc.wantError {
						t.Fatal("invalid edit reached account.connect")
					}
					if _, ok := call.Params["password"]; ok {
						t.Fatal("omitted password must remain omitted")
					}
				}
			}
		})
	}
}

func TestAccountOAuthReconnectID(t *testing.T) {
	for _, provider := range []string{"gmail", "outlook"} {
		for _, email := range []string{" USER@example.com ", "other@example.com"} {
			t.Run(provider+"/"+email, func(t *testing.T) {
				app, writer := newMailHandlerTestApp(t,
					sidecarResponsePlan{Result: map[string]any{"accounts": []any{map[string]any{"id": "legacy", "email": "user@example.com"}}}},
					sidecarResponsePlan{Result: map[string]any{"ok": true}},
					sidecarResponsePlan{Result: map[string]any{"ok": true}},
				)
				payload := map[string]any{"id": "legacy", "email": email, "avatar_url": "/media/avatar.png", "access_token": "test-access", "refresh_token": "test-refresh"}
				var result any
				var operationError error
				if provider == "gmail" {
					result, operationError = app.accountAddGmailOAuth(payload)
				} else {
					result, operationError = app.accountAddOutlookOAuth(payload)
				}
				if operationError != nil {
					t.Fatal(operationError)
				}
				want := "legacy"
				if accountID(email) != "user@example.com" {
					want = accountID(email)
				}
				if got := writer.calls[1].Params["id"]; got != want {
					t.Fatalf("connect id = %v, want %v", got, want)
				}
				if got := writer.calls[2].Params["account"]; got != want {
					t.Fatalf("watch account = %v, want %v", got, want)
				}
				if got := result.(map[string]any)["account"].(Account).ID; got != want {
					t.Fatalf("returned id = %v, want %v", got, want)
				}
			})
		}
	}
}

func TestAccountReconnectUnavailableEngine(t *testing.T) {
	for _, provider := range []string{"password", "gmail", "outlook"} {
		for _, state := range []string{"nil", "stopped"} {
			t.Run(provider+"/"+state, func(t *testing.T) {
				app := &App{}
				if state == "stopped" {
					app.sidecar = &Sidecar{}
				}
				payload := map[string]any{"id": "legacy", "email": "user@example.com", "imap_host": "imap.example.com", "smtp_host": "smtp.example.com", "username": "user@example.com", "password": "secret", "avatar_url": "/media/avatar.png", "access_token": "access", "refresh_token": "refresh"}
				_, operationError := addAccountForTest(app, provider, payload)
				if operationError == nil || operationError.Error() != app.engineUnavailable().Error() {
					t.Fatalf("error = %v, want engine unavailable", operationError)
				}
			})
		}
	}
}

func TestAccountSaveRejectsAnotherExistingMailbox(t *testing.T) {
	for _, provider := range []string{"password", "gmail", "outlook"} {
		for _, existingID := range []string{"", "account-a"} {
			t.Run(provider+"/"+existingID, func(t *testing.T) {
				app, writer := newMailHandlerTestApp(t, sidecarResponsePlan{Result: map[string]any{"accounts": []any{
					map[string]any{"id": "account-a", "email": "a@example.com"},
					map[string]any{"id": "legacy-b", "email": "b@example.com"},
				}}})
				payload := map[string]any{"id": existingID, "email": " B@example.com ", "imap_host": "imap.example.com", "smtp_host": "smtp.example.com", "username": "b@example.com", "password": "secret", "avatar_url": "/media/avatar.png", "access_token": "access", "refresh_token": "refresh"}
				_, operationError := addAccountForTest(app, provider, payload)
				if operationError == nil || operationError.Error() != "email belongs to an existing account; edit or reconnect that account instead" {
					t.Fatalf("error = %v, want duplicate account rejection", operationError)
				}
				if len(writer.calls) != 1 || writer.calls[0].Method != "account.list" {
					t.Fatalf("calls = %#v, want only account.list", writer.calls)
				}
			})
		}
	}
}

func addAccountForTest(app *App, provider string, payload map[string]any) (any, error) {
	switch provider {
	case "gmail":
		return app.accountAddGmailOAuth(payload)
	case "outlook":
		return app.accountAddOutlookOAuth(payload)
	default:
		return app.accountAddPassword(payload)
	}
}
