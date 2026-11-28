// expected
// Event controls on class properties (IEEE 1800-2017 9.4.2): `@(obj.p)`, `@(posedge obj.b)`
// and `@(p)` in a method wake when the value changes (with the edge), also when a forked
// task writes it.
class S; int count = 3; bit b; endclass
module m;
  S s = new;
  initial begin
    #10 s.count = 2;
    #10 s.count = 1;
    #10 s.count = 0;
  end
  initial begin
    while (s.count > 0) begin
      $display("%0t count = %0d", $time, s.count);
      @(s.count);
    end
    $display("%0t done", $time);
  end
  initial begin
    #5 s.b = 1; #5 s.b = 0; #5 s.b = 1;
  end
  initial begin
    @(posedge s.b) $display("%0t pos", $time);
    @(negedge s.b) $display("%0t neg", $time);
    @(posedge s.b) $display("%0t pos", $time);
  end
endmodule
class S3;
  int count;
  task dec(int d); #d; count--; endtask
  task body();
    count = 3;
    for (int i = 1; i <= 3; i++) begin
      automatic int k = i;
      fork dec(k * 10); join_none
    end
    while (count > 0) begin
      $display("%0t count = %0d", $time, count);
      @(count);
    end
    $display("%0t done", $time);
  endtask
endclass
module m3;
  initial begin
    S3 s = new;
    s.body();
  end
endmodule
