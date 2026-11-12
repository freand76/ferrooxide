// automatic tasks and functions: per-call storage, so concurrent calls from fork branches do not
// share arguments or locals (unlike static tasks).
module tb;
  task automatic say(input integer t, input [7:0] v);
    reg [7:0] local_v;
    begin
      local_v = v + 1;
      #t $display("%0t: say %0d (local %0d)", $time, v, local_v);
    end
  endtask
  function automatic integer sq(input integer x);
    integer y;
    begin
      y = x * x;
      sq = y;
    end
  endfunction
  initial begin
    fork
      say(3, 30);
      say(1, 10);
      say(2, 20);
    join
    $display("%0d %0d %0d", sq(3), sq(4) + sq(5), sq(sq(2)));
  end
endmodule
