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
    private readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Extended };
    private readonly TextBlock empty = new()
    {
        TextWrapping = TextWrapping.Wrap,
        TextAlignment = TextAlignment.Center,
        HorizontalAlignment = HorizontalAlignment.Center,
        VerticalAlignment = VerticalAlignment.Center,
        Margin = new Thickness(16),
        IsHitTestVisible = false,
        Foreground = Cards.Brush("TextFillColorSecondaryBrush"),
    };
    private readonly string placeholder;
    private readonly Action<IReadOnlyList<string>> onChange;
    private readonly Action<bool> onEditingChanged;

    /// <summary>The rows as last shown, the marked ones, and the explained one's (short, full) message.</summary>
    private IReadOnlyList<string> rows = [];
    private IReadOnlySet<int> problemRows = new HashSet<int>();
    private IReadOnlyDictionary<int, (string Text, string Help)> messages = new Dictionary<int, (string, string)>();
    private bool canAdd;

    /// <summary>The row being typed (rows.Count for a new one), its text before, and the box it is typed in.</summary>
    private int? editingRow;
    private string? editingOriginal;
    private TextBox? editor;
    private bool cancelsEdit;

    /// <summary>Rows that arrived while a row was typed; shown when the edit ends.</summary>
    private (IReadOnlyList<string> Rows, IReadOnlySet<int> Problems, IReadOnlyDictionary<int, (string, string)> Messages)? pending;

    public AddressList(string emptyText, string placeholder, double minHeight, string addTitle, string addHelp, string removeTitle,
        Action<IReadOnlyList<string>> onChange, Action<bool> onEditingChanged)
    {
        this.placeholder = placeholder;
        this.onChange = onChange;
        this.onEditingChanged = onEditingChanged;
        empty.Text = emptyText;
        ScrollViewer.SetVerticalScrollMode(list, ScrollMode.Disabled);
        ScrollViewer.SetVerticalScrollBarVisibility(list, ScrollBarVisibility.Disabled);
        // The list's own scroll area takes the mouse wheel though it doesn't scroll; the pane scrolls instead.
        list.AddHandler(UIElement.PointerWheelChangedEvent, new PointerEventHandler((_, args) =>
        {
            if (args.Handled && Ancestor<ScrollViewer>(VisualTreeHelper.GetParent(View)) is { } pane)
            {
                pane.ChangeView(null, pane.VerticalOffset - args.GetCurrentPoint(list).Properties.MouseWheelDelta, null);
            }
        }), handledEventsToo: true);
        list.SelectionChanged += (_, _) => UpdateButtons();
        list.DoubleTapped += (_, args) =>
        {
            args.Handled = true;
            var source = args.OriginalSource as DependencyObject;
            // A double-click in the row being typed selects a word there.
            if (editor is not null && IsWithin(source, editor))
            {
                return;
            }
            // Elsewhere it ends that edit first, then edits the row clicked or adds one below the rows.
            var item = Ancestor<ListViewItem>(source);
            var index = item is null ? -1 : list.IndexFromContainer(item);
            CommitEdit();
            if (index >= 0 && index < rows.Count)
            {
                BeginEditing(index);
            }
            else
            {
                AddRow();
            }
        };
        list.KeyDown += (_, args) =>
        {
            if (editingRow is not null)
            {
                return;
            }
            switch (args.Key)
            {
                case VirtualKey.Delete:
                    RemoveSelected();
                    args.Handled = true;
                    break;
                case VirtualKey.Enter or VirtualKey.F2 when list.SelectedItems.Count == 1:
                    BeginEditing(list.SelectedIndex);
                    args.Handled = true;
                    break;
            }
        };

        var area = new Grid { MinHeight = minHeight, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        area.Children.Add(list);
        area.Children.Add(empty);
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
        Show(rows, problemRows, messages, canAdd: false);
    }

    /// <summary>The card with the rows.</summary>
    public Border View { get; }

    public Button Add { get; }

    public Button Remove { get; }

    public void Show(IReadOnlyList<string> newRows, IReadOnlySet<int> newProblems, IReadOnlyDictionary<int, (string Text, string Help)> newMessages, bool canAdd)
    {
        this.canAdd = canAdd;
        UpdateButtons();
        if (editingRow is not null)
        {
            pending = (newRows, newProblems, newMessages);
            return;
        }
        if (list.Items.Count > 0 && newRows.SequenceEqual(rows) && newProblems.SetEquals(problemRows) && SameMessages(newMessages))
        {
            return;
        }
        rows = newRows;
        problemRows = newProblems;
        messages = newMessages;
        Reload();
    }

    /// <summary>Ends a row being typed, as moving the focus away would, before the list is applied.</summary>
    public void CommitEdit()
    {
        if (editor is not null)
        {
            EndEditing();
        }
    }

    private bool SameMessages(IReadOnlyDictionary<int, (string Text, string Help)> other) =>
        other.Count == messages.Count && other.All(pair => messages.TryGetValue(pair.Key, out var message) && message == pair.Value);

    private void Reload()
    {
        var selected = list.SelectedItems.Select(item => list.Items.IndexOf(item)).Where(index => index < rows.Count).ToList();
        list.Items.Clear();
        for (var index = 0; index < rows.Count; index++)
        {
            list.Items.Add(Row(rows[index], problemRows.Contains(index), messages.TryGetValue(index, out var message) ? message : null));
        }
        foreach (var index in selected)
        {
            list.SelectRange(new Microsoft.UI.Xaml.Data.ItemIndexRange(index, 1));
        }
        empty.Visible(rows.Count == 0);
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
            var note = Cards.Row(mark);
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
        Add.IsEnabled = canAdd;
        Remove.IsEnabled = editingRow is null && list.SelectedItems.Count > 0;
    }

    private void AddRow()
    {
        CommitEdit();
        if (!canAdd)
        {
            return;
        }
        list.Items.Add(Row("", problem: false, message: null));
        empty.Visible(false);
        BeginEditing(rows.Count);
    }

    private void RemoveSelected()
    {
        if (editingRow is not null)
        {
            return;
        }
        var selected = list.SelectedItems.Select(item => list.Items.IndexOf(item)).ToHashSet();
        var remaining = rows.Where((_, index) => !selected.Contains(index)).ToList();
        if (remaining.Count == rows.Count)
        {
            return;
        }
        rows = remaining;
        problemRows = new HashSet<int>();
        messages = new Dictionary<int, (string, string)>();
        // The selection doesn't move on to the rows that took the removed ones' places.
        list.SelectedIndex = -1;
        Reload();
        onChange(remaining);
    }

    /// <summary>Puts a text box over the row, with its text selected.</summary>
    private void BeginEditing(int row)
    {
        if (row < 0 || row >= list.Items.Count || list.Items[row] is not ListViewItem item || item.Content is not Grid grid)
        {
            return;
        }
        list.SelectedIndex = row;
        list.ScrollIntoView(item);
        editingRow = row;
        editingOriginal = row < rows.Count ? rows[row] : null;
        grid.Children.Clear();
        editor = new TextBox { Text = editingOriginal ?? "", PlaceholderText = placeholder };
        editor.KeyDown += (_, args) =>
        {
            if (args.Key == VirtualKey.Enter)
            {
                args.Handled = true;
                EndEditing();
                list.Focus(FocusState.Keyboard);
            }
            else if (args.Key == VirtualKey.Escape)
            {
                args.Handled = true;
                cancelsEdit = true;
                EndEditing();
                list.Focus(FocusState.Keyboard);
            }
        };
        // The box's own menu (Paste and the like) takes the focus for a moment; the edit goes on.
        editor.LostFocus += (_, _) =>
        {
            if (editor is { XamlRoot: { } root } && FocusManager.GetFocusedElement(root) is DependencyObject focused && InOpenPopup(focused, root))
            {
                return;
            }
            EndEditing();
        };
        grid.Children.Add(editor);
        editor.Loaded += (_, _) =>
        {
            editor?.Focus(FocusState.Programmatic);
            editor?.SelectAll();
        };
        UpdateButtons();
        onEditingChanged(true);
    }

    private void EndEditing()
    {
        if (editingRow is not { } row || editor is null)
        {
            return;
        }
        var text = editor.Text.Trim();
        var original = editingOriginal;
        var cancelled = cancelsEdit;
        editingRow = null;
        editingOriginal = null;
        editor = null;
        cancelsEdit = false;
        // Rows that changed meanwhile may have moved the edited row; then it is found by its text.
        var rowMoved = false;
        if (pending is { } waiting)
        {
            rowMoved = !waiting.Rows.SequenceEqual(rows);
            (rows, problemRows, messages) = waiting;
            pending = null;
        }
        List<string>? proposed = null;
        if (!cancelled && text.Length > 0 && text != original)
        {
            proposed = [.. rows];
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
            rows = proposed;
            problemRows = new HashSet<int>();
            messages = new Dictionary<int, (string, string)>();
        }
        Reload();
        if (proposed is not null)
        {
            onChange(proposed);
        }
        onEditingChanged(false);
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
        for (var current = element; current is not null; current = VisualTreeHelper.GetParent(current))
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
