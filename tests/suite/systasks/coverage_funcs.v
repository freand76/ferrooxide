// The coverage functions (IEEE 1800-2017 20.14): no coverage is kept, so each reports
// SV_COV_NOCOV, $get_coverage 0.0; the predefined constants have their values (20.14.1).
module dut; endmodule
module t;
  int i;
  real r;
  dut u1();
  initial begin
    i = $coverage_control(`SV_COV_CHECK, `SV_COV_TOGGLE, `SV_COV_HIER, $root);
    if (i != `SV_COV_NOCOV) $display("FAILED control %0d", i);
    i = $coverage_get_max(`SV_COV_TOGGLE, `SV_COV_HIER, "dut");
    if (i != `SV_COV_NOCOV) $display("FAILED get_max");
    r = $coverage_get(`SV_COV_STATEMENT, `SV_COV_MODULE, $root.t.u1);
    if (r != 0.0) $display("FAILED get");
    if ($coverage_merge(`SV_COV_ASSERTION, "n") != 0 || $coverage_save(`SV_COV_FSM_STATE, "n") != 0) $display("FAILED merge/save");
    $coverage_control(`SV_COV_START, `SV_COV_TOGGLE, `SV_COV_HIER, "dut");
    $set_coverage_db_name("c.db");
    $load_coverage_db("c.db");
    r = $get_coverage();
    if (r != 0.0) $display("FAILED get_coverage");
    if (`SV_COV_START != 0 || `SV_COV_STOP != 1 || `SV_COV_RESET != 2 || `SV_COV_CHECK != 3 || `SV_COV_MODULE != 10 || `SV_COV_HIER != 11
        || `SV_COV_ASSERTION != 20 || `SV_COV_FSM_STATE != 21 || `SV_COV_STATEMENT != 22 || `SV_COV_TOGGLE != 23
        || `SV_COV_OVERFLOW != -2 || `SV_COV_ERROR != -1 || `SV_COV_NOCOV != 0 || `SV_COV_OK != 1 || `SV_COV_PARTIAL != 2) $display("FAILED macros");
    $display("PASSED");
  end
endmodule
