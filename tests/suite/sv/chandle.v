// chandle (IEEE 1800-2017 6.14): variables, arguments and results, null, equality and use as
// a condition.
module t;
  chandle h, g;
  function chandle mk(); return null; endfunction
  function automatic bit is_null(chandle c); return c == null; endfunction
  initial begin
    int bad = 0;
    h = null;
    g = mk();
    if (!(h == null && g == null && h == g)) bad++;
    if (h) bad++;
    if (!is_null(h)) bad++;
    if (h != null) bad++;
    if (bad) $display("FAILED"); else $display("PASSED");
  end
endmodule
