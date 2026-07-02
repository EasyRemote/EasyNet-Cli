// EasyNet Windows tray companion
// ==============================
//
// File: platforms/windows/EasyNetTray/Program.cs
// Description: WPF + tray companion for EasyNet daemon status and
//              EasyNet clipboard-history promotion.
//
// Protocol Responsibility:
// - Reads local EasyNet context history from %USERPROFILE%\.easynet\context.
// - Writes only to the operator's Windows clipboard.
//
// Implementation Approach:
// - WinForms NotifyIcon provides the taskbar tray affordance.
// - RegisterHotKey provides a process-global summon shortcut.
// - WPF renders the clipboard history popup.
// - Clipboard list uses newest-to-oldest JSONL scan plus Dictionary
//   lookup, avoiding timestamp sorting.
//
// Usage Contract:
// - Run as the logged-in Windows user that owns the EasyNet state dir.
// - Shortcut is Control + Alt + V.
//
// Architectural Position:
// - Local UI facade. It does not own daemon lifecycle, capture, or
//   EasyNet persistence.

using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using DrawingIcon = System.Drawing.Icon;
using DrawingSystemIcons = System.Drawing.SystemIcons;
using Forms = System.Windows.Forms;
using MediaColor = System.Windows.Media.Color;

namespace EasyNetTray;

internal static class Program
{
    [STAThread]
    private static void Main()
    {
        var app = new Application
        {
            ShutdownMode = ShutdownMode.OnExplicitShutdown,
        };

        using var controller = new TrayController(app);
        app.Run();
    }
}

internal sealed class TrayController : IDisposable
{
    private readonly Application _app;
    private readonly ClipboardHistoryStore _store = new();
    private readonly Forms.NotifyIcon _notifyIcon;
    private readonly Forms.ToolStripMenuItem _daemonStatusItem;
    private readonly ClipboardHistoryWindow _historyWindow;
    private readonly GlobalHotKey _hotKey;
    private readonly DispatcherTimer _statusTimer;

    public TrayController(Application app)
    {
        _app = app;
        _historyWindow = new ClipboardHistoryWindow(_store);

        _daemonStatusItem = new Forms.ToolStripMenuItem("Daemon: checking...")
        {
            Enabled = false,
        };

        var menu = new Forms.ContextMenuStrip();
        menu.Items.Add(_daemonStatusItem);
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add("Show Clipboard History", null, (_, _) => ShowHistory());
        menu.Items.Add("Use Latest Clip", null, (_, _) => UseLatest());
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add("Shortcut: Ctrl + Alt + V").Enabled = false;
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add("Quit EasyNet Tray", null, (_, _) => Quit());

        _notifyIcon = new Forms.NotifyIcon
        {
            Icon = LoadIcon(),
            ContextMenuStrip = menu,
            Text = "EasyNet",
            Visible = true,
        };
        _notifyIcon.DoubleClick += (_, _) => ShowHistory();

        _hotKey = new GlobalHotKey(ModifierKeys.Control | ModifierKeys.Alt, Key.V, ToggleHistory);

        _statusTimer = new DispatcherTimer
        {
            Interval = TimeSpan.FromSeconds(3),
        };
        _statusTimer.Tick += (_, _) => UpdateDaemonStatus();
        _statusTimer.Start();
        UpdateDaemonStatus();
    }

    public void Dispose()
    {
        _statusTimer.Stop();
        _hotKey.Dispose();
        _notifyIcon.Visible = false;
        _notifyIcon.Dispose();
        _historyWindow.Close();
    }

    private static DrawingIcon LoadIcon()
    {
        var iconPath = Path.Combine(AppContext.BaseDirectory, "Resources", "easynet.ico");
        return File.Exists(iconPath) ? new DrawingIcon(iconPath) : DrawingSystemIcons.Application;
    }

    private static bool DaemonRunning()
    {
        return Process.GetProcessesByName("easynet-daemon").Length > 0;
    }

    private void UpdateDaemonStatus()
    {
        var running = DaemonRunning();
        _daemonStatusItem.Text = running ? "Daemon: running" : "Daemon: stopped";
        _notifyIcon.Text = running
            ? "EasyNet is running in the background"
            : "EasyNet daemon is not running";
    }

    private void ToggleHistory()
    {
        if (_historyWindow.IsVisible)
        {
            _historyWindow.Hide();
        }
        else
        {
            ShowHistory();
        }
    }

    private void ShowHistory()
    {
        _historyWindow.Reload();
        _historyWindow.ShowNearTaskbar();
    }

    private void UseLatest()
    {
        var latest = _store.ListSummaries(1).FirstOrDefault();
        if (latest is null)
        {
            System.Media.SystemSounds.Beep.Play();
            return;
        }

        if (!_store.ApplyToClipboard(latest.Entry))
        {
            System.Media.SystemSounds.Beep.Play();
        }
    }

    private void Quit()
    {
        Dispose();
        _app.Shutdown();
    }
}

internal sealed class ClipboardHistoryWindow : Window
{
    private readonly ClipboardHistoryStore _store;
    private readonly ListBox _list = new();
    private readonly TextBlock _status = new();
    private List<ClipSummary> _clips = [];

    public ClipboardHistoryWindow(ClipboardHistoryStore store)
    {
        _store = store;

        Title = "EasyNet Clipboard";
        Width = 520;
        Height = 420;
        WindowStyle = WindowStyle.None;
        ResizeMode = ResizeMode.NoResize;
        ShowInTaskbar = false;
        Topmost = true;
        Background = new SolidColorBrush(MediaColor.FromRgb(246, 247, 249));
        Deactivated += (_, _) => Hide();

        var root = new Border
        {
            BorderBrush = new SolidColorBrush(MediaColor.FromRgb(210, 214, 220)),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(14),
            Background = Background,
            Child = BuildLayout(),
        };
        Content = root;

        KeyDown += (_, e) =>
        {
            if (e.Key == Key.Escape)
            {
                Hide();
                e.Handled = true;
            }
            else if (e.Key == Key.Enter)
            {
                UseSelected();
                e.Handled = true;
            }
        };
    }

    public void Reload()
    {
        _clips = _store.ListSummaries();
        _list.ItemsSource = _clips;
        _list.SelectedIndex = _clips.Count > 0 ? 0 : -1;
        _status.Text = _clips.Count == 0
            ? "No EasyNet clipboard history yet."
            : $"{_clips.Count} unique items. Double-click or press Enter to move one to the Windows clipboard.";
    }

    public void ShowNearTaskbar()
    {
        var workArea = SystemParameters.WorkArea;
        Left = Math.Max(16, workArea.Right - Width - 18);
        Top = Math.Max(16, workArea.Bottom - Height - 18);
        Show();
        Activate();
        _list.Focus();
    }

    private UIElement BuildLayout()
    {
        var title = new TextBlock
        {
            Text = "EasyNet Clipboard",
            FontSize = 18,
            FontWeight = FontWeights.SemiBold,
            Foreground = new SolidColorBrush(MediaColor.FromRgb(22, 25, 29)),
        };

        _status.Margin = new Thickness(0, 4, 0, 12);
        _status.Foreground = new SolidColorBrush(MediaColor.FromRgb(90, 96, 106));
        _status.FontSize = 12;

        _list.BorderThickness = new Thickness(0);
        _list.Background = Brushes.Transparent;
        _list.ItemTemplate = BuildClipTemplate();
        _list.MouseDoubleClick += (_, _) => UseSelected();

        var panel = new DockPanel();
        DockPanel.SetDock(title, Dock.Top);
        DockPanel.SetDock(_status, Dock.Top);
        panel.Children.Add(title);
        panel.Children.Add(_status);
        panel.Children.Add(_list);
        return panel;
    }

    private static DataTemplate BuildClipTemplate()
    {
        var root = new FrameworkElementFactory(typeof(Border));
        root.SetValue(Border.MarginProperty, new Thickness(0, 0, 0, 8));
        root.SetValue(Border.PaddingProperty, new Thickness(10));
        root.SetValue(Border.CornerRadiusProperty, new CornerRadius(6));
        root.SetValue(Border.BackgroundProperty, Brushes.White);

        var grid = new FrameworkElementFactory(typeof(Grid));
        grid.SetValue(Grid.HorizontalAlignmentProperty, HorizontalAlignment.Stretch);

        var preview = new FrameworkElementFactory(typeof(TextBlock));
        preview.SetValue(TextBlock.FontSizeProperty, 13.0);
        preview.SetValue(TextBlock.FontWeightProperty, FontWeights.Medium);
        preview.SetValue(TextBlock.TextWrappingProperty, TextWrapping.NoWrap);
        preview.SetValue(TextBlock.TextTrimmingProperty, TextTrimming.CharacterEllipsis);
        preview.SetBinding(TextBlock.TextProperty, new System.Windows.Data.Binding("Entry.Preview"));

        var meta = new FrameworkElementFactory(typeof(TextBlock));
        meta.SetValue(TextBlock.MarginProperty, new Thickness(0, 20, 0, 0));
        meta.SetValue(TextBlock.FontSizeProperty, 11.0);
        meta.SetValue(TextBlock.ForegroundProperty, new SolidColorBrush(MediaColor.FromRgb(102, 111, 124)));
        meta.SetBinding(TextBlock.TextProperty, new System.Windows.Data.Binding("MetaText"));

        root.AppendChild(grid);
        grid.AppendChild(preview);
        grid.AppendChild(meta);

        return new DataTemplate
        {
            VisualTree = root,
        };
    }

    private void UseSelected()
    {
        if (_list.SelectedItem is not ClipSummary summary)
        {
            System.Media.SystemSounds.Beep.Play();
            return;
        }

        if (_store.ApplyToClipboard(summary.Entry))
        {
            Hide();
        }
        else
        {
            _status.Text = "Could not move that item to the Windows clipboard.";
            System.Media.SystemSounds.Beep.Play();
        }
    }
}

internal sealed class ClipboardHistoryStore
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNameCaseInsensitive = true,
    };

    private readonly string _contextDir;

    public ClipboardHistoryStore()
    {
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        _contextDir = Path.Combine(home, ".easynet", "context");
    }

    public List<ClipSummary> ListSummaries(int limit = 200)
    {
        var logPath = Path.Combine(_contextDir, "clipboard.jsonl");
        if (!File.Exists(logPath))
        {
            return [];
        }

        var summaries = new List<ClipSummary>();
        var positions = new Dictionary<string, int>(StringComparer.Ordinal);
        var lines = File.ReadAllLines(logPath);

        for (var i = lines.Length - 1; i >= 0; i--)
        {
            if (string.IsNullOrWhiteSpace(lines[i]))
            {
                continue;
            }

            ClipEntry? entry;
            try
            {
                entry = JsonSerializer.Deserialize<ClipEntry>(lines[i], JsonOptions);
            }
            catch (JsonException)
            {
                continue;
            }

            if (entry is null)
            {
                continue;
            }

            var key = ContentKey(entry);
            if (positions.TryGetValue(key, out var index))
            {
                summaries[index].OccurrenceCount++;
            }
            else
            {
                positions[key] = summaries.Count;
                summaries.Add(new ClipSummary(entry));
            }
        }

        return summaries.Take(Math.Clamp(limit, 1, 200)).ToList();
    }

    public bool ApplyToClipboard(ClipEntry entry)
    {
        try
        {
            if (entry.Kind == "text" && !string.IsNullOrEmpty(entry.Text))
            {
                Clipboard.SetText(entry.Text);
                return true;
            }

            if (entry.Kind == "image" && !string.IsNullOrEmpty(entry.ImageFile))
            {
                var path = Path.Combine(_contextDir, "clips", entry.ImageFile);
                if (!File.Exists(path))
                {
                    return false;
                }

                var image = new BitmapImage();
                image.BeginInit();
                image.CacheOption = BitmapCacheOption.OnLoad;
                image.UriSource = new Uri(path, UriKind.Absolute);
                image.EndInit();
                image.Freeze();
                Clipboard.SetImage(image);
                return true;
            }
        }
        catch (ExternalException)
        {
            return false;
        }
        catch (IOException)
        {
            return false;
        }

        return false;
    }

    private string ContentKey(ClipEntry entry)
    {
        using var sha = SHA256.Create();
        var input = new MemoryStream();
        input.Write(Encoding.UTF8.GetBytes(entry.Kind));
        input.WriteByte(0);

        if (!string.IsNullOrEmpty(entry.Text))
        {
            input.Write(Encoding.UTF8.GetBytes(entry.Text));
        }
        else if (!string.IsNullOrEmpty(entry.ImageFile))
        {
            input.Write(Encoding.UTF8.GetBytes(entry.ImageFile));
        }
        else
        {
            input.Write(Encoding.UTF8.GetBytes(entry.Preview));
        }

        return Convert.ToHexString(sha.ComputeHash(input.ToArray()));
    }
}

internal sealed class ClipEntry
{
    [JsonPropertyName("id")]
    public string Id { get; init; } = "";

    [JsonPropertyName("timestamp")]
    public string Timestamp { get; init; } = "";

    [JsonPropertyName("device")]
    public string Device { get; init; } = "";

    [JsonPropertyName("kind")]
    public string Kind { get; init; } = "";

    [JsonPropertyName("text")]
    public string? Text { get; init; }

    [JsonPropertyName("image_file")]
    public string? ImageFile { get; init; }

    [JsonPropertyName("preview")]
    public string Preview { get; init; } = "";
}

internal sealed class ClipSummary(ClipEntry entry)
{
    public ClipEntry Entry { get; } = entry;
    public int OccurrenceCount { get; set; } = 1;
    public int DuplicateCount => Math.Max(0, OccurrenceCount - 1);

    public string MetaText
    {
        get
        {
            var count = DuplicateCount > 0 ? $"  x{OccurrenceCount}" : "";
            return $"{Entry.Kind}  {CompactTime(Entry.Timestamp)}{count}";
        }
    }

    private static string CompactTime(string raw)
    {
        if (!DateTimeOffset.TryParse(raw, out var parsed))
        {
            return raw;
        }

        var local = parsed.ToLocalTime();
        return local.Date == DateTimeOffset.Now.Date
            ? local.ToString("HH:mm:ss")
            : local.ToString("MM-dd HH:mm");
    }
}

internal sealed class GlobalHotKey : IDisposable
{
    private const int WmHotKey = 0x0312;
    private const uint ModAlt = 0x0001;
    private const uint ModControl = 0x0002;

    private readonly int _id = 0x454E;
    private readonly Action _callback;
    private readonly Window _messageWindow;
    private HwndSource? _source;
    private bool _registered;

    public GlobalHotKey(ModifierKeys modifiers, Key key, Action callback)
    {
        _callback = callback;
        _messageWindow = new Window
        {
            Width = 1,
            Height = 1,
            Left = -32000,
            Top = -32000,
            WindowStyle = WindowStyle.None,
            ShowInTaskbar = false,
            ShowActivated = false,
            Opacity = 0,
        };
        _messageWindow.SourceInitialized += (_, _) => Register(modifiers, key);
        _messageWindow.Show();
    }

    public void Dispose()
    {
        if (_registered && _source is not null)
        {
            UnregisterHotKey(_source.Handle, _id);
        }
        _source?.RemoveHook(WndProc);
        _messageWindow.Close();
    }

    private void Register(ModifierKeys modifiers, Key key)
    {
        _source = HwndSource.FromHwnd(new WindowInteropHelper(_messageWindow).Handle);
        _source?.AddHook(WndProc);

        var nativeModifiers = 0u;
        if (modifiers.HasFlag(ModifierKeys.Control))
        {
            nativeModifiers |= ModControl;
        }
        if (modifiers.HasFlag(ModifierKeys.Alt))
        {
            nativeModifiers |= ModAlt;
        }

        var virtualKey = KeyInterop.VirtualKeyFromKey(key);
        if (_source is not null)
        {
            _registered = RegisterHotKey(_source.Handle, _id, nativeModifiers, (uint)virtualKey);
        }
    }

    private IntPtr WndProc(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == WmHotKey && wParam.ToInt32() == _id)
        {
            _callback();
            handled = true;
        }

        return IntPtr.Zero;
    }

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool RegisterHotKey(IntPtr hWnd, int id, uint fsModifiers, uint vk);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool UnregisterHotKey(IntPtr hWnd, int id);
}
