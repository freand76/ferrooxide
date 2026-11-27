// A method function's join_none fork branches keep the arguments and variables of the
// call that started them, also when the same process calls it again before they read
// them (IEEE 1800-2017 9.3.2, 13.4.4), as UVM's process guards do.
module top;
  int seen[$];
  class W;
    int id;
    function new(int i);
      id = i;
    endfunction
    static function void watch(W w, int d);
      int twice = w.id * 2;
      fork
        begin
          #d;
          seen.push_back(w.id * 100 + twice);
        end
      join_none
    endfunction
  endclass
  W ws[2];
  initial begin
    ws[0] = new(1);
    ws[1] = new(2);
    // (one call, made twice)
    for (int i = 0; i < 2; i++)
      W::watch(ws[i], 3 - 2 * i);
    #5;
    if (seen.size() == 2 && seen[0] == 204 && seen[1] == 102) $display("PASSED");
    else $display("FAILED: %p", seen);
  end
endmodule
