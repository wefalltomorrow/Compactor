/* jshint strict: true, esversion: 5, browser: true */

var Util = (function() {
	"use strict";

	var powers = '_KMGTPEZY';
	var monotime = function() { return Date.now(); };

	if (window.performance && window.performance.now)
		monotime = function() { return window.performance.now(); };

	return {
		debounce: function(callback, delay) {
			var timeout;
			var fn = function() {
				var context = this;
				var args = arguments;

				clearTimeout(timeout);
				timeout = setTimeout(function() {
					timeout = null;
					callback.apply(context, args);
				}, delay);
			};
			fn.clear = function() {
				clearTimeout(timeout);
				timeout = null;
			};

			return fn;
		},

		throttle: function(callback, delay) {
			var timeout;
			var last;
			var fn = function() {
				var context = this;
				var args = arguments;
				var now = monotime();

				if (last && now < last + delay) {
					clearTimeout(timeout);
					timeout = setTimeout(function() {
						timeout = null;
						last = now;
						callback.apply(context, args);
					}, delay);
				} else {
					last = now;
					callback.apply(context, args);
				}
			};
			fn.clear = function() {
				clearTimeout(timeout);
				timeout = null;
			};

			return fn;
		},

		format_number: function(number, digits) {
			if (digits === undefined) digits = 2;
			return number.toLocaleString("en", {minimumFractionDigits: digits, maximumFractionDigits: digits});
		},

		bytes_to_human_dec: function(bytes) {
			for (var i = powers.length - 1; i > 0; i--) {
				var div = Math.pow(10, 3 * i);
				if (bytes >= div) {
					return Util.format_number(bytes / div, 2) + " " + powers[i] + 'B';
				}
			}

			return Util.format_number(bytes, 0) + ' B';
		},

		bytes_to_human_bin: function(bytes) {
			for (var i = powers.length - 1; i > 0; i--) {
				var div = Math.pow(2, 10*i);
				if (bytes >= div) {
					return Util.format_number(bytes / div, 2) + " " + powers[i] + 'iB';
				}
			}

			return Util.format_number(bytes, 0) + ' B';
		},

		human_to_bytes: function(human) {
			if (!human) return null;
			var num = parseFloat(human);

			var match = (/\s*([KMGTPEZY])(i)?([Bb])?\s*$/i).exec(human);
			if (match) {
				var pow = (match[2] == 'i') ? 1024 : 1000;
				num *= Math.pow(pow, powers.indexOf(match[1].toUpperCase()));
			}

			return num;
		},

		number_to_human: function(num) {
			for (var i = powers.length - 1; i > 0; i--) {
				var div = Math.pow(10, 3*i);
				if (num >= div) {
					return Util.format_number(num / div, 2) + powers[i];
				}
			}

			return num;
		},

		human_to_number: function(human) {
			if (!human) return null;
			var num = parseFloat(human);

			var match = (/\s*([KMGTPEZY])\s*$/i).exec(human);

			if (match) {
				num *= Math.pow(1000, powers.indexOf(match[1].toUpperCase()));
			}

			return num;
		},

		sformat: function() {
			var args = arguments;
			return args[0].replace(/\{(\d+)\}/g, function (m, n) { return args[parseInt(n, 10) + 1]; });
		},

		range: function(a, b, step) {
			if (!step) step = 1;
			var arr = [];
			for (var i = a; i < b; i += step) {
				arr.push(i);
			}

			return arr;
		}
	};
})();

Util.bytes_to_human = Util.bytes_to_human_bin;

var Action = (function() {
	"use strict";

	return {
		open_url: function(url) {
			external.invoke(JSON.stringify({ type: 'OpenUrl', url: url }));
		},

		reset_config: function() {
			external.invoke(JSON.stringify({ type: 'ResetConfig' }));
		},

		save_config: function(config) {
			config.type = 'SaveConfig';
			external.invoke(JSON.stringify(config));
		},

		choose_folder: function() {
			external.invoke(JSON.stringify({ type: 'ChooseFolder' }));
		},

		compress: function() {
			external.invoke(JSON.stringify({ type: 'Compress' }));
		},

		decompress: function() {
			external.invoke(JSON.stringify({ type: 'Decompress' }));
		},

		view_compressed: function(view, query, page) {
			external.invoke(JSON.stringify({
				type: 'ViewCompressed',
				view: view,
				query: query,
				page: page
			}));
		},

		pause: function() {
			external.invoke(JSON.stringify({ type: 'Pause' }));
		},

		resume: function() {
			external.invoke(JSON.stringify({ type: 'Resume' }));
		},

		analyse: function() {
			external.invoke(JSON.stringify({ type: 'Analyse' }));
		},

		stop: function() {
			external.invoke(JSON.stringify({ type: 'Stop' }));
		},

		quit: function() {
			external.invoke(JSON.stringify({ type: 'Quit' }));
		}
	};
})();

var Response = (function() {
	"use strict";

	return {
		dispatch: function(msg) {
			switch(msg.type) {
				case "Config":
					Gui.set_decimal(msg.decimal);
					Gui.set_compression(msg.compression);
					Gui.set_min_savings(msg.min_savings_percent);
					Gui.set_max_threads(msg.max_threads);
					Gui.set_hdd_single_thread(msg.hdd_single_thread);
					Gui.set_excludes(msg.excludes);
					break;

				case "Folder":
					Gui.set_folder(msg.path);
					break;

				case "Version":
					Gui.version(msg.date, msg.version);
					break;

				case "Status":
					Gui.set_status(msg.status, msg.pct);
					break;

				case "Error":
					window.alert(msg.title + "\n\n" + msg.message);
					break;

				case "Paused":
				case "Resumed":
				case "Stopped":
				case "Scanned":
				case "Compacting":
					Gui[msg.type.toLowerCase()]();
					break;

				case "FolderSummary":
					Gui.set_folder_summary(msg.info);
					break;

				case "CompressedView":
					Gui.set_compressed_view(msg);
					break;
			}
		}
	};
})();

var Gui = (function() {
	"use strict";

	var compressedView = {
		view: "folders",
		page: 0,
		pages: 1
	};

	var compressedSearch = Util.debounce(function() {
		Gui.request_compressed(0);
	}, 200);

	var addTableCell = function(row, value, className) {
		var cell = $("<td></td>").text(value);
		if (className) cell.addClass(className);
		row.append(cell);
	};

	return {
		boot: function() {
			$("a[href]").on("click", function(e) {
				e.preventDefault();
				Action.open_url($(this).attr("href"));
				return false;
			});

			$("#Button_Save").on("click", function() {
				var minSavings = parseFloat($("#Min_Savings").val());
				if (isNaN(minSavings)) minSavings = 1;
				var maxThreads = parseInt($("#Max_Threads").val(), 10);
				if (isNaN(maxThreads)) maxThreads = 0;

				Action.save_config({
					decimal: $("#SI_Units").val() == "D",
					compression: $("#Compression_Mode").val(),
					min_savings_percent: minSavings,
					max_threads: maxThreads,
					hdd_single_thread: $("#HDD_Single_Thread").prop("checked"),
					excludes: $("#Excludes").val()
				});
			});

			$("#Button_Reset").on("click", function() {
				Action.reset_config();
			});

			$("#Compressed_View_Search").on("input", compressedSearch);
		},

		page: function(page) {
			$("nav button").removeClass("active");
			$("#Button_Page_" + page).addClass("active");
			$("section.page").hide();
			$("#" + page).show();
		},

		open_compressed_view: function() {
			compressedView.view = "folders";
			compressedView.page = 0;
			compressedView.pages = 1;
			$("#Compressed_View_Search").val("");
			Gui.page("CompressedFiles");
			Gui.request_compressed(0);
		},

		request_compressed: function(page) {
			var query = $("#Compressed_View_Search").val() || "";
			var requestedPage = parseInt(page, 10);
			if (isNaN(requestedPage) || requestedPage < 0) requestedPage = 0;
			Action.view_compressed(compressedView.view, query, requestedPage);
		},

		compressed_page: function(delta) {
			Gui.request_compressed(compressedView.page + delta);
		},

		set_compressed_view: function(data) {
			compressedView.view = "folders";
			compressedView.page = data.page;
			compressedView.pages = data.pages;

			$("#Compressed_View_Root").text(data.root);
			$("#Compressed_View_Summary").text(
				Util.format_number(data.compressed_count, 0) + " compressed files - " +
				Util.bytes_to_human(data.logical_size) + " logical - " +
				Util.bytes_to_human(data.physical_size) + " on-disk - " +
				Util.bytes_to_human(Math.max(0, data.logical_size - data.physical_size)) + " saved"
			);

			var head = $("#Compressed_View_Head").empty();
			var body = $("#Compressed_View_Body").empty();
			var header = $("<tr></tr>");

			addTableCell(header, "Files");
			addTableCell(header, "Logical");
			addTableCell(header, "On-disk");
			addTableCell(header, "Saved");
			addTableCell(header, "Folder", "path");
			head.append(header);

			data.items.forEach(function(item) {
				var row = $("<tr></tr>");
				addTableCell(row, Util.format_number(item.count, 0));
				addTableCell(row, Util.bytes_to_human(item.logical_size));
				addTableCell(row, Util.bytes_to_human(item.physical_size));
				addTableCell(row, Util.bytes_to_human(Math.max(0, item.logical_size - item.physical_size)));
				addTableCell(row, item.path, "path");
				body.append(row);
			});

			if (data.items.length === 0) {
				var empty = $("<tr></tr>");
				empty.append(
					$("<td></td>")
						.attr("colspan", 5)
						.addClass("empty")
						.text("No matching compressed folders")
				);
				body.append(empty);
			}

			$("#Compressed_View_Page").text(
				"Page " + Util.format_number(data.page + 1, 0) + " of " +
				Util.format_number(data.pages, 0) + " - " +
				Util.format_number(data.total, 0) + " folders"
			);
			$("#Button_Compressed_Previous").prop("disabled", data.page <= 0);
			$("#Button_Compressed_Next").prop("disabled", data.page + 1 >= data.pages);
		},

		version: function(date, version) {
			$(".compile-date").text(date);
			$(".version").text(version);
		},

		set_decimal: function(dec) {
			var field = $("#SI_Units");
			if (dec) {
				field.val("D");
				Util.bytes_to_human = Util.bytes_to_human_dec;
			} else {
				field.val("I");
				Util.bytes_to_human = Util.bytes_to_human_bin;
			}
		},

		set_compression: function(compression) {
			$("#Compression_Mode").val(compression);
		},

		set_min_savings: function(percent) {
			$("#Min_Savings").val(percent);
		},

		set_max_threads: function(threads) {
			$("#Max_Threads").val(threads);
		},

		set_hdd_single_thread: function(enabled) {
			$("#HDD_Single_Thread").prop("checked", enabled);
		},

		set_excludes: function(excludes) {
			$("#Excludes").val(excludes);
		},

		set_folder: function(folder) {
			var bits = folder.split(/:\\|\\/).map(function(x) { return document.createTextNode(x); });
			var end = bits.pop();

			var button = $("#Button_Folder");
			button.empty();
			bits.forEach(function(bit) {
				button.append(bit);
				button.append($("<span>&gt;</span>"));
			});
			button.append(end);

			Gui.scanning();
		},

		set_status: function(status, pct) {
			$("#Activity_Text").text(status);
			if (pct != null) {
				$("#Activity_Progress").val(pct);
			} else {
				$("#Activity_Progress").removeAttr("value");
			}
		},

		scanning: function() {
			Gui.reset_folder_summary();
			$("#Activity").show();
			$("#Analysis").show();
			$("#Button_Pause").show();
			$("#Button_Resume").hide();
			$("#Button_Stop").show();
			$("#Button_Analyse").hide();
			$("#Button_Compress").hide();
			$("#Button_Decompress").hide();
			$("#Button_View_Compressed").hide();
			$("#Command").show();
		},

		compacting: function() {
			$("#Button_Pause").show();
			$("#Button_Resume").hide();
			$("#Button_Stop").show();
			$("#Button_Analyse").hide();
			$("#Button_Compress").hide();
			$("#Button_Decompress").hide();
			$("#Button_View_Compressed").hide();
		},

		paused: function() {
			$("#Button_Pause").hide();
			$("#Button_Resume").show();
		},

		resumed: function() {
			$("#Button_Pause").show();
			$("#Button_Resume").hide();
		},

		stopped: function() {
			Gui.scanned();
		},

		scanned: function() {
			$("#Button_Pause").hide();
			$("#Button_Resume").hide();
			$("#Button_Stop").hide();
			$("#Button_Analyse").show();

			if ($("#File_Count_Compressible").text() != "0") {
				$("#Button_Compress").show();
			} else {
				$("#Button_Compress").hide();
			}

			if ($("#File_Count_Compressed").text() != "0") {
				$("#Button_Decompress").show();
				$("#Button_View_Compressed").show();
			} else {
				$("#Button_Decompress").hide();
				$("#Button_View_Compressed").hide();
			}
		},

		reset_folder_summary: function() {
			Gui.set_folder_summary({
				logical_size: 0,
				physical_size: 0,
				direct_storage: false,
				compressed: {count: 0, logical_size: 0, physical_size: 0, estimated_physical_size: 0},
				compressible: {count: 0, logical_size: 0, physical_size: 0, estimated_physical_size: 0},
				skipped: {count: 0, logical_size: 0, physical_size: 0, estimated_physical_size: 0}
			});
		},

		set_folder_summary: function(data) {
			$("#Size_Logical").text(Util.bytes_to_human(data.logical_size));
			$("#Size_Physical").text(Util.bytes_to_human(data.physical_size));

			if (data.logical_size > 0) {
				var ratio = (data.physical_size / data.logical_size);
				$("#Compress_Ratio").text(Util.format_number(ratio, 2));
			} else {
				$("#Compress_Ratio").text("1.00");
			}

			$("#Compressed_Size").text(Util.bytes_to_human(data.compressed.physical_size));
			$("#Compressible_Size").text(Util.bytes_to_human(data.compressible.physical_size));
			$("#Skipped_Size").text(Util.bytes_to_human(data.skipped.physical_size));

			if (data.physical_size > 0) {
				var total = data.physical_size;
				document.getElementById("Breakdown_Compressed").style.width = "" + 100 * (data.compressed.physical_size / total).toFixed(2) + "%";
				document.getElementById("Breakdown_Compressible").style.width = "" + 100 * (data.compressible.physical_size / total).toFixed(2) + "%";
				document.getElementById("Breakdown_Skipped").style.width = "" + 100 * (data.skipped.physical_size / total).toFixed(2) + "%";
			}

			var estimatedCandidateSize = data.compressible.estimated_physical_size;
			if (estimatedCandidateSize === undefined || estimatedCandidateSize === null) {
				estimatedCandidateSize = data.compressible.physical_size;
			}
			var estimatedSavings = Math.max(0, data.compressible.physical_size - estimatedCandidateSize);
			var estimatedTotal = Math.max(0, data.physical_size - estimatedSavings);

			$("#Space_Saved").text(Util.bytes_to_human(Math.max(0, data.compressed.logical_size - data.compressed.physical_size)));
			$("#Estimated_Savings").text(Util.bytes_to_human(estimatedSavings));
			$("#Estimated_Physical").text(Util.bytes_to_human(estimatedTotal));

			$("#File_Count_Compressed").text(Util.format_number(data.compressed.count, 0));
			$("#File_Count_Compressible").text(Util.format_number(data.compressible.count, 0));
			$("#File_Count_Skipped").text(Util.format_number(data.skipped.count, 0));
			$("#DirectStorage_Warning").toggle(!!data.direct_storage);
		},

		analysis_complete: function() {
			$("#Activity").hide();
			$("#Analysis").show();
		}
	};
})();

$(document).ready(Gui.boot);
