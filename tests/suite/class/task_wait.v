// A method task that waits, called on two objects from two processes at the same time:
// each call keeps its own object across the waits.
class Acc;
  int total;
  string tag;
  function new(string t);
    tag = t;
  endfunction
  task add(int v, int n);
    repeat (n) begin
      #1 total += v;
    end
  endtask
endclass

module m;
  Acc a = new("a"), b = new("b");
  initial a.add(1, 5);
  initial b.add(10, 3);
  initial begin
    #10;
    if (a.total != 5 || b.total != 30 || a.tag != "a") $display("FAILED -- %0d %0d", a.total, b.total);
    else $display("PASSED");
  end
endmodule
