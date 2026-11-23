using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// One list of the Exception Rules (components/ui/macos/AddressListEditor.swift): a card of address
/// rows with + and − under it. A double-click on a row, Enter or F2 edits it in place; a double-click
/// on the empty part of the card or + adds a row; Delete or − removes the selected rows. Every finished
/// edit reports the whole list; nothing is saved here. The list grows with its rows instead of
/// scrolling inside the scrolling pane.
/// </summary>
internal sealed class AddressList
{
    private readonly ListView _list = new() { SelectionMode = ListViewSelectionMode.Extended };
    private readonly TextBlock _empty = new()
    {
        TextWrapping = TextWrapping.Wrap,
        TextAlignment = TextAlignment.Center,
        HorizontalAlignment = HorizontalAlignment.Center,
        VerticalAlignment = VerticalAlignment.Center,
        Margin = new Thickness(16),
        IsHitTestVisible = false,
        Foreground = Cards.Brush("TextFillColorSecondaryBrush"),
    };
    private readonly string _placeholder;
    private readonly Action<IReadOnlyList<string>> _onChange;

    /// <summary>The rows as last shown, the marked ones, and the explained one's (short, full) message.</summary>
    private IReadOnlyList<string> _rows = [];
    private IReadOnlySet<int> _problemRows = new HashSet<int>();
    private IReadOnlyDictionary<int, (string Text, string Help)> _messages = new Dictionary<int, (string, string)>();
    private bool _canAdd;

    /// <summary>The row being typed (rows.Count for a new one), its text before, and the box it is typed in.</summary>
    private int? _editingRow;
    private string? _editingOriginal;
    private TextBox? _editor;
    private bool _cancelsEdit;

    /// <summary>Rows that arrived while a row was typed; shown when the edit ends.</summary>
    private (IReadOnlyList<string> Rows, IReadOnlySet<int> Problems, IReadOnlyDictionary<int, (string, string)> Messages)? _pending;

    public AddressList(string emptyText, string placeholder, double minHeight, string addTitle, string addHelp, string removeTitle,
        Action<IReadOnlyList<string>> onChange)
    {
        this._placeholder = placeholder;
        this._onChange = onChange;
        _empty.Text = emptyText;
        ScrollViewer.SetVerticalScrollMode(_list, ScrollMode.Disabled);
        ScrollViewer.SetVerticalScrollBarVisibility(_list, ScrollBarVisibility.Disabled);
        // The list's own scroll area takes the mouse wheel though it doesn't scroll; the pane scrolls instead.
        _list.AddHandler(UIElement.PointerWheelChangedEvent, new PointerEventHandler((_, args) =>
        {
            if (args.Handled && Ancestor<ScrollViewer>(VisualTreeHelper.GetParent(View)) is { } pane)
            {
                _ = pane.ChangeView(null, pane.VerticalOffset - args.GetCurrentPoint(_list).Properties.MouseWheelDelta, null);
            }
        }), handledEventsToo: true);
        _list.SelectionChanged += (_, _) => UpdateButtons();
        _list.DoubleTapped += (_, args) =>
        {
            args.Handled = true;
            var source = args.OriginalSource as DependencyObject;
            // A double-click in the row being typed selects a word there.
            if (_editor is not null && IsWithin(source, _editor))
            {
                return;
            }
            // Elsewhere it ends that edit first, then edits the row clicked or adds one below the rows.
            ListViewItem? item = Ancestor<ListViewItem>(source);
            int index = item is null ? -1 : _list.IndexFromContainer(item);
            CommitEdit();
            if (index >= 0 && index < _rows.Count)
            {
                BeginEditing(index);
            }
            else
            {
                AddRow();
            }
        };
        _list.KeyDown += (_, args) =>
        {
            if (_editingRow is not null)
            {
                return;
            }
            switch (args.Key)
            {
                case VirtualKey.Delete:
                    RemoveSelected();
                    args.Handled = true;
                    break;
                case VirtualKey.Enter or VirtualKey.F2 when _list.SelectedItems.Count == 1:
                    BeginEditing(_list.SelectedIndex);
                    args.Handled = true;
                    break;
            }
        };

        var area = new Grid { MinHeight = minHeight, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        area.Children.Add(_list);
        area.Children.Add(_empty);
        // Below the last row, a double-click adds one, as on the empty list.
        area.DoubleTapped += (_, args) =>
        {
            if (!args.Handled)
            {
                AddRow();
            }
        };
        View = new Border { Style = (Style)Application.Current.Resources["ListCardStyle"], Child = area };

        Add = IconButton("\uE710", addTitle, addHelp);
        Add.Click += (_, _) => AddRow();
        Remove = IconButton("\uE738", removeTitle, L("Remove the selected rows"));
        Remove.Click += (_, _) => RemoveSelected();
        Show(_rows, _problemRows, _messages, canAdd: false);
    }

    /// <summary>The card with the rows.</summary>
    public Border View { get; }

    public Button Add { get; }

    public Button Remove { get; }

    public void Show(IReadOnlyList<string> newRows, IReadOnlySet<int> newProblems, IReadOnlyDictionary<int, (string Text, string Help)> newMessages, bool canAdd)
    {
        this._canAdd = canAdd;
        UpdateButtons();
        if (_editingRow is not null)
        {
            _pending = (newRows, newProblems, newMessages);
            return;
        }
        if (_list.Items.Count > 0 && newRows.SequenceEqual(_rows) && newProblems.SetEquals(_problemRows) && SameMessages(newMessages))
        {
            return;
        }
        _rows = newRows;
        _problemRows = newProblems;
        _messages = newMessages;
        Reload();
    }

    /// <summary>Ends a row being typed, as moving the focus away would, before the list is applied.</summary>
    public void CommitEdit()
    {
        if (_editor is not null)
        {
            EndEditing();
        }
    }

    private bool SameMessages(IReadOnlyDictionary<int, (string Text, string Help)> other) =>
        other.Count == _messages.Count && other.All(pair => _messages.TryGetValue(pair.Key, out (string Text, string Help) message) && message == pair.Value);

    private void Reload()
    {
        var selected = _list.SelectedItems.Select(_list.Items.IndexOf).Where(index => index < _rows.Count).ToList();
        _list.Items.Clear();
        for (int index = 0; index < _rows.Count; index++)
        {
            _list.Items.Add(Row(_rows[index], _problemRows.Contains(index), _messages.TryGetValue(index, out (string Text, string Help) message) ? message : null));
        }
        foreach (int index in selected)
        {
            _list.SelectRange(new Microsoft.UI.Xaml.Data.ItemIndexRange(index, 1));
        }
        _empty.Visible(_rows.Count == 0);
        UpdateButtons();
    }

    private static ListViewItem Row(string text, bool problem, (string Text, string Help)? message)
    {
        var grid = new Grid { ColumnSpacing = 8 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.Children.Add(new TextBlock { Text = text, VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis });
        if (problem)
        {
            var mark = new FontIcon { FontSize = 14 };
            mark.SetSucceeded(false);
            StackPanel note = Cards.Row(mark);
            note.Spacing = 6;
            if (message is { } shown)
            {
                note.Children.Add(new TextBlock { Text = shown.Text, VerticalAlignment = VerticalAlignment.Center, Foreground = Cards.Brush("SystemFillColorCriticalBrush") });
                ToolTipService.SetToolTip(note, shown.Help);
            }
            Grid.SetColumn(note, 1);
            grid.Children.Add(note);
        }
        return new ListViewItem { Content = grid, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    }

    private void UpdateButtons()
    {
        Add.IsEnabled = _canAdd;
        Remove.IsEnabled = _editingRow is null && _list.SelectedItems.Count > 0;
    }

    private void AddRow()
    {
        CommitEdit();
        if (!_canAdd)
        {
            return;
        }
        _list.Items.Add(Row("", problem: false, message: null));
        _empty.Visible(false);
        BeginEditing(_rows.Count);
    }

    private void RemoveSelected()
    {
        if (_editingRow is not null)
        {
            return;
        }
        var selected = _list.SelectedItems.Select(_list.Items.IndexOf).ToHashSet();
        var remaining = _rows.Where((_, index) => !selected.Contains(index)).ToList();
        if (remaining.Count == _rows.Count)
        {
            return;
        }
        _rows = remaining;
        _problemRows = new HashSet<int>();
        _messages = new Dictionary<int, (string, string)>();
        // The selection doesn't move on to the rows that took the removed ones' places.
        _list.SelectedIndex = -1;
        Reload();
        _onChange(remaining);
    }

    /// <summary>Puts a text box over the row, with its text selected.</summary>
    private void BeginEditing(int row)
    {
        if (row < 0 || row >= _list.Items.Count || _list.Items[row] is not ListViewItem item || item.Content is not Grid grid)
        {
            return;
        }
        _list.SelectedIndex = row;
        _list.ScrollIntoView(item);
        _editingRow = row;
        _editingOriginal = row < _rows.Count ? _rows[row] : null;
        grid.Children.Clear();
        _editor = new TextBox { Text = _editingOriginal ?? "", PlaceholderText = _placeholder };
        _editor.KeyDown += (_, args) =>
        {
            if (args.Key == VirtualKey.Enter)
            {
                args.Handled = true;
                EndEditing();
                _ = _list.Focus(FocusState.Keyboard);
            }
            else if (args.Key == VirtualKey.Escape)
            {
                args.Handled = true;
                _cancelsEdit = true;
                EndEditing();
                _ = _list.Focus(FocusState.Keyboard);
            }
        };
        // The box's own menu (Paste and the like) takes the focus for a moment; the edit goes on.
        _editor.LostFocus += (_, _) =>
        {
            if (_editor is { XamlRoot: { } root } && FocusManager.GetFocusedElement(root) is DependencyObject focused && InOpenPopup(focused, root))
            {
                return;
            }
            EndEditing();
        };
        grid.Children.Add(_editor);
        _editor.Loaded += (_, _) =>
        {
            _ = (_editor?.Focus(FocusState.Programmatic));
            _editor?.SelectAll();
        };
        UpdateButtons();
    }

    private void EndEditing()
    {
        if (_editingRow is not { } row || _editor is null)
        {
            return;
        }
        string text = _editor.Text.Trim();
        string? original = _editingOriginal;
        bool cancelled = _cancelsEdit;
        _editingRow = null;
        _editingOriginal = null;
        _editor = null;
        _cancelsEdit = false;
        // Rows that changed meanwhile may have moved the edited row; then it is found by its text.
        bool rowMoved = false;
        if (_pending is { } waiting)
        {
            rowMoved = !waiting.Rows.SequenceEqual(_rows);
            (_rows, _problemRows, _messages) = waiting;
            _pending = null;
        }
        List<string>? proposed = null;
        if (!cancelled && text.Length > 0 && text != original)
        {
            proposed = [.. _rows];
            if (original is null)
            {
                proposed.Add(text);
            }
            else if (!rowMoved && row < proposed.Count && proposed[row] == original)
            {
                proposed[row] = text;
            }
            else if (proposed.IndexOf(original) is var index and >= 0)
            {
                proposed[index] = text;
            }
            else
            {
                proposed.Add(text);
            }
            _rows = proposed;
            _problemRows = new HashSet<int>();
            _messages = new Dictionary<int, (string, string)>();
        }
        Reload();
        if (proposed is not null)
        {
            _onChange(proposed);
        }
    }

    private static Button IconButton(string glyph, string name, string help)
    {
        var button = new Button { Content = new FontIcon { Glyph = glyph, FontSize = 14 }, Width = 36, Height = 32, Padding = new Thickness(0) };
        ToolTipService.SetToolTip(button, help);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, name);
        return button;
    }

    private static bool InOpenPopup(DependencyObject element, XamlRoot root) =>
        VisualTreeHelper.GetOpenPopupsForXamlRoot(root).Any(popup => popup.Child is { } child && IsWithin(element, child));

    private static bool IsWithin(DependencyObject? element, DependencyObject container)
    {
        for (DependencyObject? current = element; current is not null; current = VisualTreeHelper.GetParent(current))
        {
            if (current == container)
            {
                return true;
            }
        }
        return false;
    }

    private static T? Ancestor<T>(DependencyObject? element) where T : DependencyObject
    {
        while (element is not null and not T)
        {
            element = VisualTreeHelper.GetParent(element);
        }
        return element as T;
    }
}
