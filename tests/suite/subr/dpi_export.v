// `export "DPI-C"` makes a function callable from foreign code (IEEE
// 1800-2017 35.7); without foreign code it changes nothing.
export "DPI-C" function unit_f;
function int unit_f(int x);
  return x + 1;
endfunction

module top;
  export "DPI-C" function f;
  export "DPI-C" c_name = task t;
  function int f(int x);
    return 2 * x;
  endfunction
  task t(output int y);
    y = 7;
  endtask
  initial begin
    int y;
    t(y);
    if (f(3) == 6 && y == 7 && unit_f(1) == 2) $display("PASSED");
    else $display("FAILED");
  end
endmodule
